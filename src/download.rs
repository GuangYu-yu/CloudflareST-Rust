use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::cmp::min;
use std::collections::VecDeque;

use reqwest::Client;
use url;

use crate::progress::Bar;
use crate::args::Args;
use crate::PingResult;
use crate::common;

// 指数加权移动平均
struct Ewma {
    value: f64,
    alpha: f64,
    initialized: bool,
}

impl Ewma {
    fn new(alpha: f64) -> Self {
        Self {
            value: 0.0,
            alpha,
            initialized: false,
        }
    }

    fn add(&mut self, value: f64) {
        if !self.initialized {
            self.value = value;
            self.initialized = true;
        } else {
            self.value = self.alpha * value + (1.0 - self.alpha) * self.value;
        }
    }

    fn value(&self) -> f64 {
        self.value
    }
}

// 定义下载处理器来处理下载数据
struct DownloadHandler {
    data_received: u64,
    headers: std::collections::HashMap<String, String>,
    last_update: Instant,
    current_speed: Arc<Mutex<f64>>,
    start_time: Instant,
    time_slice: Duration,
    next_slice: Instant,
    last_content_read: u64,
    time_counter: u32,
    speed_samples: VecDeque<(Instant, u64)>,
    ewma: Ewma,
}

impl DownloadHandler {
    fn new(current_speed: Arc<Mutex<f64>>) -> Self {
        let now = Instant::now();
        Self {
            data_received: 0,
            headers: std::collections::HashMap::new(),
            last_update: now,
            current_speed,
            start_time: now,
            time_slice: Duration::from_millis(100),
            next_slice: now + Duration::from_millis(100),
            last_content_read: 0,
            time_counter: 1,
            speed_samples: VecDeque::new(),
            ewma: Ewma::new(0.3),
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
                let time_diff = last.0.duration_since(first.0).as_secs_f64();

                if time_diff > 0.0 {
                    bytes_diff as f64 / time_diff
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

        // --- EWMA 计算部分 ---
        // 时间片计算 - 使用EWMA计算平均速度
        let current_time = Instant::now();
        
        if current_time >= self.next_slice {
            // 计算这个时间片内的下载量
            let content_diff = self.data_received - self.last_content_read;
            
            // 添加到EWMA中
            self.ewma.add(content_diff as f64);
            
            // 更新计数器和下一个时间片
            self.time_counter += 1;
            self.next_slice = self.start_time + self.time_slice * self.time_counter;
            self.last_content_read = self.data_received;
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
    test_count: usize,
    min_speed: f64,
    tcp_port: u16,
    bar: Arc<Bar>,
    current_speed: Arc<Mutex<f64>>,
    httping: bool,
    colo_filter: String,
    ping_results: Vec<PingResult>,
}

// 按下载速度（降序）、延迟（升序）、丢包率（升序）
fn sort_ping_results(results: &mut Vec<PingResult>) {
    results.sort_by(|a, b| {
        let (a_speed, a_loss, a_delay) = common::extract_ping_metrics(a);
        let (b_speed, b_loss, b_delay) = common::extract_ping_metrics(b);
        match b_speed.partial_cmp(&a_speed).unwrap() {
            std::cmp::Ordering::Equal => {
                match a_delay.partial_cmp(&b_delay).unwrap() {  // 先比较延迟
                    std::cmp::Ordering::Equal => {
                        a_loss.partial_cmp(&b_loss).unwrap()  // 最后比较丢包率
                    },
                    other => other,
                }
            },
            other => other,
        }
    });
}

impl DownloadTest {
    pub async fn new(args: &Args, ping_results: Vec<PingResult>) -> Self {
        let url = args.url.clone();
        let urlist = args.urlist.clone();
        let timeout = args.timeout_duration;
        let test_count = args.test_count;
        let min_speed = args.min_speed;
        let tcp_port = args.tcp_port;
        let httping = args.httping;
        let colo_filter = args.httping_cf_colo.clone();
        
        // 使用 common 模块获取 URL 列表
        let urlist_vec = common::get_url_list(&url, &urlist).await;

        // 计算实际需要测试的数量
        let test_num = min(test_count, ping_results.len());
        
        Self {
            url,
            urlist: urlist_vec,
            timeout,
            test_count,
            min_speed,
            tcp_port,
            bar: Arc::new(Bar::new(test_num as u64, "", "")),
            current_speed: Arc::new(Mutex::new(0.0)),
            httping,
            colo_filter,
            ping_results,
        }
    }

    pub async fn test_download_speed(&mut self) -> (Vec<PingResult>, bool) {
        // 先检查队列数量是否足够
        if self.test_count > self.ping_results.len() {
            println!("\n[信息] {}", "队列数量不足所需数量！");
        }

        println!("开始下载测速（下限：{:.2} MB/s, 所需：{}, 队列：{}）", 
                self.min_speed, self.test_count, self.ping_results.len());
        
        // 记录符合要求的结果索引
        let mut qualified_indices = Vec::new();
        
        // 数据中心过滤条件
        let colo_filters = common::parse_colo_filters(&self.colo_filter);
        
        // 创建一个任务来更新进度条的速度显示
        let current_speed: Arc<Mutex<f64>> = Arc::clone(&self.current_speed);
        let bar: Arc<Bar> = Arc::clone(&self.bar);
        let speed_update_handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let speed = *current_speed.lock().unwrap();
                if speed > 0.0 {
                    bar.as_ref().set_suffix(format!("{:.2} MB/s", speed / 1024.0 / 1024.0));
                }
            }
        });
    
        // 逐个IP进行测速（单线程）
        for i in 0..self.ping_results.len() {
            // 使用引用
            let ping_result = &mut self.ping_results[i];
            
            // 获取IP地址和检查是否需要获取 colo
            let (ip, need_colo) = if self.httping {
                let PingResult::Http(data) = ping_result else { unreachable!() };
                (data.ip, data.data_center.is_empty())
            } else {
                let PingResult::Tcp(data) = ping_result else { unreachable!() };
                (data.ip, data.data_center.is_empty())
            };
            
            // 执行下载测速
            let (speed, maybe_colo) = if !self.urlist.is_empty() {
                // 使用 urlist 中的 URL，根据 IP 索引选择不同的 URL 进行轮询
                let url_index = i % self.urlist.len();
                let test_url = &self.urlist[url_index];
                
                download_handler(
                    ip, 
                    test_url, 
                    self.timeout.unwrap(),
                    Arc::clone(&self.current_speed), 
                    self.tcp_port, 
                    need_colo
                ).await
            } else {
                // 使用单一 URL 进行测速
                download_handler(
                    ip, 
                    &self.url, 
                    self.timeout.unwrap(),
                    Arc::clone(&self.current_speed), 
                    self.tcp_port, 
                    need_colo
                ).await
            };
            
            // 无论速度如何，都更新下载速度和可能的数据中心信息
            if self.httping {
                if let PingResult::Http(data) = ping_result {
                    if common::process_download_result(
                        data, 
                        speed, 
                        maybe_colo, 
                        self.min_speed, 
                        &colo_filters
                    ) {
                        qualified_indices.push(i);
                        self.bar.as_ref().grow(1, "".to_string());
                    }
                }
            } else {
                if let PingResult::Tcp(data) = ping_result {
                    if common::process_download_result(
                        data, 
                        speed, 
                        maybe_colo, 
                        self.min_speed, 
                        &colo_filters
                    ) {
                        qualified_indices.push(i);
                        self.bar.as_ref().grow(1, "".to_string());
                    }
                }
            }
            
            // 如果已经找到足够数量的合格结果，提前结束测试
            if qualified_indices.len() >= self.test_count {
                break;
            }
        }
    
        // 中止速度更新任务
        speed_update_handle.abort();
        
        // 完成进度条
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



// 构建reqwest客户端
async fn build_reqwest_client(ip: IpAddr, url: &str, port: u16, timeout: Duration) -> Option<Client> {
    common::build_reqwest_client(ip, url, port, timeout).await
}

// 下载测速处理函数
async fn download_handler(
    ip: IpAddr, 
    url: &str, 
    download_duration: Duration,
    current_speed: Arc<Mutex<f64>>,
    tcp_port: u16,
    need_colo: bool
) -> (f64, Option<String>) {
    
    // 解析原始URL以获取主机名和路径
    let url_parts = match url::Url::parse(url) {
        Ok(parts) => parts,
        Err(_) => return (0.0, None),
    };
    
    let host = match url_parts.host_str() {
        Some(host) => host,
        None => return (0.0, None),
    };
    
    let path = url_parts.path();
    let is_https = url_parts.scheme() == "https";
    let mut data_center = None;
    
    // 创建客户端进行下载测速
    let client = match build_reqwest_client(ip, url, tcp_port, download_duration).await {
        Some(client) => client,
        None => return (0.0, None),
    };
    
    // 创建下载处理器
    let mut handler = DownloadHandler::new(Arc::clone(&current_speed));
    
    // 使用公共模块发送请求
    let response = common::send_request(&client, is_https, host, tcp_port, path, "GET").await;
    
    // 如果获取到响应，开始下载
    let avg_speed = if let Some(mut resp) = response {
        // 更新头部信息
        handler.update_headers(resp.headers());
        
        // 如果需要获取数据中心信息，从响应头中提取
        if need_colo {
            data_center = common::extract_data_center(&resp);
        }
        
        // 使用reqwest的内置超时功能
        // 设置一个取消点，当达到下载时间时取消
        let cancel_at = tokio::time::Instant::now() + download_duration;
        
        // 读取响应体
        loop {
            // 检查是否超时
            if tokio::time::Instant::now() >= cancel_at {
                break;
            }
            
            // 获取下一个数据块
            match tokio::time::timeout(Duration::from_secs(1), resp.chunk()).await {
                Ok(Ok(Some(chunk))) => {
                    let size = chunk.len() as u64;
                    handler.update_data_received(size);
                },
                Ok(Ok(None)) => break, // 数据读取完毕
                _ => break, // 出错或超时
            }
        }

        let now = Instant::now();
        // 确保 time_counter > 0 避免 usize 减法溢出
        if handler.time_counter > 0 {
            // 计算上一个完整时间片的结束时间点
            let last_slice_end_time = handler.start_time + handler.time_slice * (handler.time_counter - 1);
            // 计算最后一个不完整时间片的实际持续时间
            let last_slice_duration = now.duration_since(last_slice_end_time);
            // 计算最后一个不完整时间片下载的数据量
            let last_content_diff = handler.data_received - handler.last_content_read;

            if last_content_diff > 0 && last_slice_duration.as_secs_f64() > 0.0 {
                // 计算实际持续时间与标准时间片的比例
                let time_ratio = last_slice_duration.as_secs_f64() / handler.time_slice.as_secs_f64();
                if time_ratio > 0.0 {
                    // 根据比例调整数据量并添加到EWMA
                    handler.ewma.add(last_content_diff as f64 / time_ratio);
                } else {
                     // 如果时间比例过小或为0，直接添加原始数据量
                     handler.ewma.add(last_content_diff as f64);
                }
            } else if last_content_diff > 0 {
                 // 如果时间差为0但有数据，直接添加
                 handler.ewma.add(last_content_diff as f64);
            }
        }
        
        // 使用EWMA计算的平均速度
        let final_ewma_value = handler.ewma.value();
        let time_factor = download_duration.as_secs_f64() / 120.0;

        if time_factor > 0.0 {
             final_ewma_value / time_factor
        } else {
             0.0 // 避免除以零
        }
    } else {
        0.0
    };
    
    // 重置当前速度显示
    *current_speed.lock().unwrap() = 0.0;
    
    (avg_speed, data_center)
}