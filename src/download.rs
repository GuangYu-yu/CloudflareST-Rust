use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::cmp::min;

use reqwest::Client;
use url;

use crate::progress::Bar;
use crate::args::Args;
use crate::PingResult;
use crate::common;

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
    // 用于计算速度
    last_bytes: u64,
    last_time: Instant,
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
            // 初始化新字段
            last_bytes: 0,
            last_time: now,
        }
    }

    fn update_data_received(&mut self, size: u64) {
        self.data_received += size;
        
        // 获取当前时间和计算时间差
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update);
        
        // 500毫秒更新一次速度
        if elapsed.as_millis() >= 500 {
            let current_bytes = self.data_received;
            let time_diff = now.duration_since(self.last_time).as_secs_f64();
            
            if time_diff > 0.0 {
                let bytes_diff = current_bytes - self.last_bytes;
                let speed = bytes_diff as f64 / time_diff;
                
                // 更新当前速度显示
                *self.current_speed.lock().unwrap() = speed;
            }
            
            // 更新上次记录的字节数和时间
            self.last_bytes = current_bytes;
            self.last_time = now;
            
            // 重置计时器
            self.last_update = now;
        }
        
        // 时间片计算
        let current_time = Instant::now();
        
        if current_time >= self.next_slice {
            self.time_counter += 1;
            self.next_slice = self.start_time + self.time_slice * self.time_counter;
            self.last_content_read = self.data_received;
        }
    }

    fn get_data_received(&self) -> u64 {
        self.data_received
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
    urlist: Vec<String>,  // Vec<String> 存储多个 URL
    timeout: Option<Duration>,
    test_count: usize,
    min_speed: f64,
    tcp_port: u16,
    bar: Arc<Bar>,
    current_speed: Arc<Mutex<f64>>, // 跟踪当前速度
    httping: bool,
    colo_filter: String,
}

impl DownloadTest {
    pub async fn new(args: &Args) -> Self {
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
        
        Self {
            url,
            urlist: urlist_vec,
            timeout,
            test_count,
            min_speed,
            tcp_port,
            bar: Arc::new(Bar::new(0, "", "")),
            current_speed: Arc::new(Mutex::new(0.0)),
            httping,
            colo_filter,
        }
    }

    pub async fn test_download_speed(&self, mut ping_results: Vec<PingResult>) -> Vec<PingResult> {
        // 获取测试数量，不超过结果集大小
        let test_num = min(self.test_count, ping_results.len());
        
        // 队列数量不足
        if self.test_count > ping_results.len() {
            println!("\n[信息] {}", "队列数量不足所需数量！");
        }
        
        println!("开始下载测速（下限：{:.2} MB/s, 数量：{}, 队列：{}）", 
                 self.min_speed, test_num, ping_results.len());
        
        // 记录符合要求的结果索引
        let mut qualified_indices = Vec::new();
        
        // 数据中心过滤条件
        let colo_filters = common::parse_colo_filters(&self.colo_filter);
        
        // 创建一个任务来更新进度条的速度显示
        let current_speed = Arc::clone(&self.current_speed);
        let bar = Arc::clone(&self.bar);
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
        for i in 0..test_num {
            // 使用引用
            let ping_result = &mut ping_results[i];
            
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
            
            // 更新进度条
            self.bar.as_ref().grow(1, "".to_string());
            
            // 无论速度如何，都更新下载速度和可能的数据中心信息
            if self.httping {
                if let PingResult::Http(data) = ping_result {
                    data.download_speed = speed;
                    // 如果数据中心为空且获取到了新的数据中心信息，则更新
                    if data.data_center.is_empty() {
                        if let Some(colo) = maybe_colo {
                            data.data_center = colo;
                        }
                    }
                    
                    // 检查速度是否符合要求
                    let speed_match = speed >= self.min_speed * 1024.0 * 1024.0;
                    
                    // 如果设置了 colo 过滤条件，需要同时满足速度和数据中心要求
                    if !colo_filters.is_empty() {
                        // 检查数据中心是否符合过滤条件
                        let colo_match = !data.data_center.is_empty() && 
                        (colo_filters.is_empty() || colo_filters.iter().any(|filter| data.data_center.to_uppercase() == *filter));
                        
                        // 同时满足速度和数据中心要求才添加到合格索引
                        if speed_match && colo_match {
                            qualified_indices.push(i);
                        }
                    } else {
                        // 如果没有设置 colo 过滤条件，只需要满足速度要求
                        if speed_match {
                            qualified_indices.push(i);
                        }
                    }
                }
            } else {
                if let PingResult::Tcp(data) = ping_result {
                    data.download_speed = speed;
                    // 同上
                    if data.data_center.is_empty() {
                        if let Some(colo) = maybe_colo {
                            data.data_center = colo;
                        }
                    }
                    
                    // 检查速度是否符合要求
                    let speed_match = speed >= self.min_speed * 1024.0 * 1024.0;
                    
                    // 如果设置了 colo 过滤条件，需要同时满足速度和数据中心要求
                    if !colo_filters.is_empty() {
                        // 检查数据中心是否符合过滤条件
                        let colo_match = !data.data_center.is_empty() && 
                        (colo_filters.is_empty() || colo_filters.iter().any(|filter| data.data_center.to_uppercase() == *filter));
                        
                        // 同时满足速度和数据中心要求才添加到合格索引
                        if speed_match && colo_match {
                            qualified_indices.push(i);
                        }
                    } else {
                        // 如果没有设置 colo 过滤条件，只需要满足速度要求
                        if speed_match {
                            qualified_indices.push(i);
                        }
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
        
        // 如果没有符合速度要求的结果，返回原始集合
        if qualified_indices.is_empty() {
            println!("没有符合要求的 IP，返回全部结果");
            return ping_results;
        }
        
        // 筛选出合格的结果
        let mut qualified_results = Vec::new();
        for &idx in &qualified_indices {
            qualified_results.push(ping_results[idx].clone());
        }
        
        // 按下载速度（降序）、丢包率（升序）、延迟（升序）排序
        qualified_results.sort_by(|a, b| {
            let (a_speed, a_loss, a_delay) = common::extract_ping_metrics(a);
            let (b_speed, b_loss, b_delay) = common::extract_ping_metrics(b);
            
            // 先按下载速度降序排序
            match b_speed.partial_cmp(&a_speed).unwrap() {
                std::cmp::Ordering::Equal => {
                    // 如果下载速度相同，按丢包率升序排序
                    match a_loss.partial_cmp(&b_loss).unwrap() {
                        std::cmp::Ordering::Equal => {
                            // 如果丢包率也相同，按延迟升序排序
                            a_delay.partial_cmp(&b_delay).unwrap()
                        },
                        other => other,
                    }
                },
                other => other,
            }
        });
        
        qualified_results
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
    
    // 记录开始时间
    let start_time = Instant::now();
    
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
        
        // 计算平均速度
        let elapsed = start_time.elapsed();
        if elapsed.as_secs_f64() > 0.0 {
            handler.get_data_received() as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        }
    } else {
        0.0
    };
    
    // 重置当前速度显示
    *current_speed.lock().unwrap() = 0.0;
    
    (avg_speed, data_center)
}