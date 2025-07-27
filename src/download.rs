use std::net::SocketAddr;
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

        // 检查是否需要更新显示速度
        let elapsed_since_last_update = now.duration_since(self.last_update);
        if elapsed_since_last_update.as_millis() >= 500 {
        // 通过取队列中第一个和最后一个数据点计算字节差和时间差
        // 若没有数据或时间差无效，速度返回0
        let speed = self.speed_samples
            .front()
            .zip(self.speed_samples.back())
            .and_then(|(first, last)| {
                let bytes_diff = last.1 - first.1;
                let time_diff = last.0.duration_since(first.0).as_secs_f32();
                if bytes_diff == 0 || time_diff <= 0.0 {
                    None
                } else {
                    Some(bytes_diff as f32 / time_diff)
                }
            })
            .unwrap_or(0.0);

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
    bar: Arc<Bar>,
    current_speed: Arc<Mutex<f32>>,
    colo_filter: Arc<Vec<Arc<str>>>,
    ping_results: Vec<PingData>,
    timeout_flag: Arc<AtomicBool>,
}

impl DownloadTest {
    pub async fn new(args: &Args, ping_results: Vec<PingData>, timeout_flag: Arc<AtomicBool>) -> Self {
        // 使用 common 模块获取 URL 列表
        let urlist_vec = common::get_url_list(&args.url, &args.urlist).await;

        // 计算实际需要测试的数量
        let test_num = min(args.test_count as u32, ping_results.len() as u32);
        
        // 先检查队列数量是否足够
        if args.test_count as usize > ping_results.len() {
            println!("[信息] 队列的 IP 数量不足，可能需要降低延迟测速筛选条件！");
        }

        println!("开始下载测速（下限：{:.2} MB/s, 所需：{}, 队列：{}）", 
                args.min_speed, args.test_count, ping_results.len());
        
        Self {
            url: args.url.to_string(),
            urlist: urlist_vec,
            timeout: args.timeout_duration,
            test_count: args.test_count,
            min_speed: args.min_speed,
            bar: Arc::new(Bar::new(test_num as u64, "", "")),
            current_speed: Arc::new(Mutex::new(0.0)),
            colo_filter: Arc::new(common::parse_colo_filters(&args.httping_cf_colo)),
            ping_results,
            timeout_flag,
        }
    }

    pub async fn test_download_speed(&mut self) -> Vec<PingData> {
        // 数据中心过滤条件
        let colo_filters = Arc::clone(&self.colo_filter);
        
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

        let mut ping_queue = self.ping_results.drain(..).collect::<VecDeque<_>>();
        let mut qualified_results = Vec::with_capacity(self.test_count as usize);
        let mut tested_count = 0;

        while let Some(mut ping_result) = ping_queue.pop_front() {
            // 检查是否收到超时信号或已经找到足够数量的合格结果
            if common::check_timeout_signal(&self.timeout_flag)
                || qualified_results.len() >= self.test_count as usize
            {
                break;
            }

            // 获取IP地址和检查是否需要获取 colo
            let need_colo = ping_result.data_center.is_empty();

            // 执行下载测速
            let test_url = if !self.urlist.is_empty() {
                let url_index = tested_count % self.urlist.len();
                &self.urlist[url_index]
            } else {
                &self.url
            };

            let (speed, maybe_colo) = download_handler(
                ping_result.addr,
                test_url,
                self.timeout.unwrap(),
                Arc::clone(&self.current_speed),
                need_colo,
                Arc::clone(&self.timeout_flag),
                Arc::clone(&colo_filters),
            ).await;

            // 更新下载速度和可能的数据中心信息
            ping_result.download_speed = speed;

            if ping_result.data_center.is_empty() {
                if let Some(colo) = maybe_colo {
                    ping_result.data_center = colo;
                }
            }

            // 检查速度是否符合要求
            let speed_match = match speed {
                Some(s) => s >= self.min_speed * 1024.0 * 1024.0,
                None => false,
            };

            // 检查数据中心是否符合要求
            let colo_match = if !colo_filters.is_empty() {
                common::is_colo_matched(&ping_result.data_center, &colo_filters)
            } else {
                true // 没有过滤条件时视为匹配
            };

            // 同时满足速度和数据中心要求
            if speed_match && colo_match {
                qualified_results.push(ping_result);
                // 更新进度条
                self.bar.as_ref().grow(1, "");
            }
            
            // 更新已测试计数
            tested_count += 1;
        }

        // 中止速度更新任务
        speed_update_handle.abort();
        
        // 完成进度条但保持当前进度
        self.bar.done_at_current_pos();

        // 如果没有找到足够的结果，打印提示
        if qualified_results.len() < self.test_count as usize {
            println!("\n[信息] 下载测速符合要求的 IP 数量不足！");
        }

        // 对结果进行业务排序
        common::sort_results(&mut qualified_results);

        qualified_results
    }
}

// 下载测速处理函数
async fn download_handler(
    addr: SocketAddr,
    url: &str, 
    download_duration: Duration,
    current_speed: Arc<Mutex<f32>>,
    need_colo: bool,
    timeout_flag: Arc<AtomicBool>,
    colo_filters: Arc<Vec<Arc<str>>>,
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

    let mut data_center = None;
    
    // 计算延长的下载时间
    let warm_up_duration = Duration::from_secs(3); // 预热时间
    let extended_duration = download_duration + warm_up_duration;

    // 创建客户端进行下载测速
    let client = match common::build_reqwest_client(addr, host, extended_duration.as_millis() as u64).await {
        Some(client) => client,
        None => return (None, None),
    };
    
    // 创建下载处理器
    let mut handler = DownloadHandler::new(Arc::clone(&current_speed));
    
    // 发送请求
    let response = client.get(url).send().await.ok();
    
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
                if !colo_filters.is_empty() && !common::is_colo_matched(dc, &colo_filters) {
                    return (None, data_center);
                }
            }
        }
        
        // 读取响应体
        let time_start = Instant::now();
        let mut actual_content_read: u64 = 0;
        let mut actual_start_time: Option<Instant> = None;
        let mut last_data_time: Option<Instant> = None; // 记录最后读取数据的时间
        
        loop {
            let current_time = Instant::now();
            let elapsed = current_time.duration_since(time_start);
            
            // 检查是否超过总下载时间或收到超时信号
            if timeout_flag.load(Ordering::SeqCst) {
                break;
            }
            
            // 读取数据块
            match resp.chunk().await.ok() {
                Some(Some(chunk)) => {
                    let size = chunk.len() as u64;
                    handler.update_data_received(size);
                    
                    // 如果已经过了预热时间，开始记录实际下载数据
                    if elapsed >= warm_up_duration {
                        if actual_start_time.is_none() {
                            actual_start_time = Some(current_time);
                        }
                        actual_content_read += size;
                        last_data_time = Some(current_time); // 更新最后数据时间
                    }
                },
                _ => break,
            }
        }
        
        // 计算实际速度（只计算预热后的数据）
        actual_start_time.and_then(|start| {
            let end_time = last_data_time.unwrap_or_else(Instant::now); // 使用最后数据时间
            let actual_elapsed = end_time.duration_since(start).as_secs_f32();
            if actual_elapsed > 0.0 {
                Some(actual_content_read as f32 / actual_elapsed)
            } else {
                None
            }
        })
    } else {
        None
    };
    
    // 重置当前速度显示
    *current_speed.lock().unwrap() = 0.0;
    
    (avg_speed, data_center)
}