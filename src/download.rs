use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::cmp::min;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};

use url;

use crate::progress::Bar;
use crate::args::Args;
use crate::common::{self, PingData};

// 定义下载处理器来处理下载数据
struct DownloadHandler {
    data_received: u64,
    headers: std::collections::HashMap<String, String>,
    last_update: Instant,
    current_speed: Arc<Mutex<f32>>,
    speed_samples: VecDeque<(Instant, u64)>,
}

impl DownloadHandler {
    fn new(current_speed: Arc<Mutex<f32>>) -> Self {
        let now = Instant::now();
        Self {
            data_received: 0,
            headers: std::collections::HashMap::new(),
            last_update: now,
            current_speed,
            speed_samples: VecDeque::new(),
        }
    }

    fn update_data_received(&mut self, size: u64) {
        self.data_received += size;
        let now = Instant::now();

        // 将当前数据点添加到队列
        self.speed_samples.push_back((now, self.data_received));

        // 移除超出 500ms 窗口的数据点
        let window_start = now - Duration::from_millis(500);
        while let Some(front) = self.speed_samples.front() {
            if front.0 < window_start {
                self.speed_samples.pop_front();
            } else {
                break; // 队列中的数据都在窗口内了
            }
        }

        // 检查是否需要更新显示速度（例如，仍然保持大约 500ms 更新一次）
        let elapsed_since_last_update = now.duration_since(self.last_update);
        if elapsed_since_last_update.as_millis() >= 500 {
            let speed = if self.speed_samples.len() >= 2 {
                // 计算窗口内的速度
                let first = self.speed_samples.front().unwrap();
                let last = self.speed_samples.back().unwrap();
                let bytes_diff = last.1 - first.1;
                let time_diff = last.0.duration_since(first.0).as_secs_f32();

                if time_diff > 0.0 {
                    bytes_diff as f32 / time_diff
                } else {
                    0.0 // 时间差为0，速度为0
                }
            } else {
                0.0 // 样本不足，速度为0
            };

            // 更新当前速度显示
            *self.current_speed.lock().unwrap() = speed;

            // 更新上次显示更新的时间
            self.last_update = now;
        }
    }

    fn update_headers(&mut self, headers: &reqwest::header::HeaderMap) {
        for (name, value) in headers.iter() {
            if let Ok(value_str) = value.to_str() {
                self.headers.insert(name.as_str().to_lowercase(), value_str.to_string());
            }
        }
    }
}

pub struct DownloadTest {
    url: String,
    urlist: Vec<String>,
    timeout: Option<Duration>,
    test_count: u16,
    min_speed: f32,
    tcp_port: u16,
    bar: Arc<Bar>,
    current_speed: Arc<Mutex<f32>>,
//    icmp_ping: bool,
    colo_filter: String,
    ping_results: Vec<PingData>,
    timeout_flag: Arc<AtomicBool>,
}

// 按下载速度（降序）、延迟（升序）、丢包率（升序）
fn sort_ping_results(results: &mut Vec<PingData>) {
    // 计算平均值
    let total_count = results.len() as f32;
    let (total_speed, total_loss, total_delay) = results.iter().fold((0.0, 0.0, 0.0), |acc, data| {
        (acc.0 + data.download_speed.unwrap_or(0.0), acc.1 + data.loss_rate(), acc.2 + data.delay)
    });

    let avg_speed = total_speed / total_count;
    let avg_loss = total_loss / total_count;
    let avg_delay = total_delay / total_count;

    // 计算分数并排序
    results.sort_by(|a, b| {
        let calculate_score = |data: &PingData| {
            let speed_diff = data.download_speed.unwrap_or(0.0) - avg_speed;
            let delay_diff = data.delay - avg_delay;
            let loss_diff = data.loss_rate() - avg_loss;
            
            speed_diff * 0.5 - delay_diff * 0.2 - loss_diff * 0.3
        };

        let a_score = calculate_score(a);
        let b_score = calculate_score(b);

        b_score.partial_cmp(&a_score).unwrap_or(std::cmp::Ordering::Equal)
    });
}

impl DownloadTest {
    pub async fn new(args: &Args, ping_results: Vec<PingData>, timeout_flag: Arc<AtomicBool>) -> Self {
        // 使用 common 模块获取 URL 列表
        let urlist_vec = common::get_url_list(&args.url, &args.urlist).await;

        // 计算实际需要测试的数量
        let test_num = min(args.test_count as u32, ping_results.len() as u32);
        
        Self {
            url: args.url.to_string(),
            urlist: urlist_vec,
            timeout: args.timeout_duration,
            test_count: args.test_count,
            min_speed: args.min_speed,
            tcp_port: args.tcp_port,
            bar: Arc::new(Bar::new(test_num as u64, "", "")),
            current_speed: Arc::new(Mutex::new(0.0)),
//            icmp_ping: args.icmp_ping,
            colo_filter: args.httping_cf_colo.to_string(),
            ping_results,
            timeout_flag,
        }
    }

    pub async fn test_download_speed(&mut self) -> (Vec<PingData>, bool) {
        // 先检查队列数量是否足够
        if self.test_count as usize > self.ping_results.len() {
            println!("\n[信息] {}", "队列数量不足所需数量！");
        }

        println!("开始下载测速（下限：{:.2} MB/s, 所需：{}, 队列：{}）", 
                self.min_speed, self.test_count, self.ping_results.len());
        
        // 记录符合要求的结果索引
        let mut qualified_indices = Vec::new();
        
        // 数据中心过滤条件
        let colo_filters = Arc::new(common::parse_colo_filters(&self.colo_filter));
        
        // 创建一个任务来更新进度条的速度显示
        let current_speed: Arc<Mutex<f32>> = Arc::clone(&self.current_speed);
        let bar: Arc<Bar> = Arc::clone(&self.bar);
        let timeout_flag_clone = Arc::clone(&self.timeout_flag);
        let speed_update_handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                // 检查是否收到超时信号
                if timeout_flag_clone.load(Ordering::SeqCst) {
                    break;
                }
                let speed = *current_speed.lock().unwrap();
                if speed > 0.0 {
                    bar.as_ref().set_suffix(format!("{:.2} MB/s", speed / 1024.0 / 1024.0));
                }
            }
        });
    
        // 逐个IP进行测速（单线程）
        for i in 0..self.ping_results.len() {
            // 检查是否收到超时信号
            if common::check_timeout_signal(&self.timeout_flag) {
                break;
            }
            
            // 使用引用
            let ping_result = &mut self.ping_results[i];
            
            // 获取IP地址和检查是否需要获取 colo
            let (ip, need_colo) = (ping_result.ip, ping_result.data_center.is_empty());
            
            // 执行下载测速
            let test_url = if !self.urlist.is_empty() {
                let url_index = i % self.urlist.len();
                &self.urlist[url_index]
            } else {
                &self.url
            };
            
            let (speed, maybe_colo) = download_handler(
                ip,
                test_url,
                self.timeout.unwrap(),
                Arc::clone(&self.current_speed),
                self.tcp_port,
                need_colo,
                Arc::clone(&self.timeout_flag),
                Arc::clone(&colo_filters),
            ).await;
            
            // 无论速度如何，都更新下载速度和可能的数据中心信息
            let process_ping_data = |data: &mut PingData| {
                if common::process_download_result(
                    data,
                    speed,
                    maybe_colo,
                    self.min_speed,
                    &colo_filters,
                ) {
                    qualified_indices.push(i);
                    self.bar.as_ref().grow(1, "");
                }
            };

            process_ping_data(ping_result);
            
            // 如果已经找到足够数量的合格结果，提前结束测试
            if qualified_indices.len() >= self.test_count as usize {
                break;
            }
        }
    
        // 中止速度更新任务
        speed_update_handle.abort();
        
        // 更新进度条为完成状态
        self.bar.done();
        
        // 返回排序后的原始集合
        if qualified_indices.is_empty() {
            sort_ping_results(&mut self.ping_results);
            return (std::mem::take(&mut self.ping_results), true);
        }

        // 筛选出合格的结果
        let mut qualified_results = Vec::new();
        for &idx in &qualified_indices {
            qualified_results.push(self.ping_results[idx].clone());
        }
        sort_ping_results(&mut qualified_results);
        (qualified_results, false) // false 表示有合格结果
    }
}

// 下载测速处理函数
async fn download_handler(
    ip: IpAddr, 
    url: &str, 
    download_duration: Duration,
    current_speed: Arc<Mutex<f32>>,
    tcp_port: u16,
    need_colo: bool,
    timeout_flag: Arc<AtomicBool>,
    colo_filters: Arc<Vec<String>>,
) -> (Option<f32>, Option<String>) {
    
    // 解析原始URL以获取主机名和路径
    let url_parts = match url::Url::parse(url) {
        Ok(parts) => parts,
        Err(_) => return (None, None),
    };
    
    let host = match url_parts.host_str() {
        Some(host) => host,
        None => return (None, None),
    };
    
    let path = url_parts.path();
    let scheme = url_parts.scheme();
    let mut data_center = None;
    
    // 创建客户端进行下载测速
    let client = match common::build_reqwest_client_with_host(ip, tcp_port, host, download_duration.as_millis() as u64).await {
        Some(client) => client,
        None => return (None, None),
    };
    
    // 创建下载处理器
    let mut handler = DownloadHandler::new(Arc::clone(&current_speed));
    
    // 发送请求
    let url_with_port = format!("{}://{}:{}{}", scheme, host, tcp_port, path);
    let response = client.get(&url_with_port).send().await.ok();
    
    // 如果获取到响应，开始下载
    let avg_speed = if let Some(mut resp) = response {
        // 更新头部信息
        handler.update_headers(resp.headers());
        
        // 如果需要获取数据中心信息，从响应头中提取
        if need_colo {
            data_center = common::extract_data_center(&resp);
            // 如果没有提取到数据中心信息，直接返回None
            if data_center.is_none() {
                return (None, None);
            }
            // 如果数据中心不符合要求，速度返回None，数据中心正常返回
            if let Some(dc) = &data_center {
                if !colo_filters.is_empty() && !colo_filters.iter().any(|f| dc.contains(f)) {
                    return (None, data_center);
                }
            }
        }
        
        // 读取响应体
        let time_start = Instant::now();
        let mut content_read: u64 = 0;
        let mut actual_content_read: u64 = 0;
        let mut actual_start_time: Option<Instant> = None;
        let warm_up_duration = Duration::from_secs(3); // 3秒预热时间
        let extended_duration = download_duration + warm_up_duration; // 延长总下载时间
        
        loop {
            let current_time = Instant::now();
            let elapsed = current_time.duration_since(time_start);
            
            // 检查是否超过总下载时间或收到超时信号
            if elapsed >= extended_duration || timeout_flag.load(Ordering::SeqCst) {
                break;
            }
            
            // 读取数据块
            match resp.chunk().await.ok() {
                Some(Some(chunk)) => {
                    let size = chunk.len() as u64;
                    content_read += size;
                    handler.update_data_received(size);
                    
                    // 如果已经过了预热时间，开始记录实际下载数据
                    if elapsed >= warm_up_duration {
                        // 如果这是第一次超过预热时间，记录实际开始时间
                        if actual_start_time.is_none() {
                            actual_start_time = Some(current_time);
                        }
                        actual_content_read += size;
                    }
                },
                _ => break,
            }
        }
        
        // 计算实际速度（只计算预热后的数据）
        if let Some(start) = actual_start_time {
            let actual_elapsed = Instant::now().duration_since(start).as_secs_f32();
            if actual_elapsed > 0.0 {
                Some(actual_content_read as f32 / actual_elapsed)
            } else {
                None
            }
        } else {
            // 如果没有记录到预热后的数据，使用总数据计算
            let elapsed = time_start.elapsed().as_secs_f32();
            if elapsed > 0.0 {
                Some(content_read as f32 / elapsed)
            } else {
                None
            }
        }
    } else {
        None
    };
    
    // 重置当前速度显示
    *current_speed.lock().unwrap() = 0.0;
    
    (avg_speed, data_center)
}