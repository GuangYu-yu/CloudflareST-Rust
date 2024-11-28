use std::net::IpAddr;
use std::time::{Duration, Instant};
use reqwest::{Client, redirect, header::HeaderMap};
use crate::types::{Config, PingDelaySet, DownloadSpeedSet, SpeedTestError};
use crate::progress::Bar;
use futures::StreamExt;
use ewma::EWMA;
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;
use rand::seq::SliceRandom;

const BUFFER_SIZE: usize = 1024;
const DEFAULT_URL: &str = "https://cf.xiu2.xyz/url";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_TEST_NUM: u32 = 10;
const DEFAULT_MIN_SPEED: f64 = 0.0;
const DELAY_GROUP_INTERVAL: Duration = Duration::from_millis(2); // 2ms 分组间隔
const MAX_RETRIES: u32 = 3; // 最大重试次数
const CLIENT_POOL_SIZE: usize = 100; // 客户端连接池大小

// 客户端连接池
pub struct DownloadClient {
    pool: Arc<Vec<Client>>,
    index: Arc<Mutex<usize>>,
}

impl DownloadClient {
    pub fn new() -> Self {
        let mut clients = Vec::with_capacity(CLIENT_POOL_SIZE);
        for _ in 0..CLIENT_POOL_SIZE {
            clients.push(Client::new());
        }
        Self {
            pool: Arc::new(clients),
            index: Arc::new(Mutex::new(0)),
        }
    }

    pub fn get_client(&self) -> Client {
        let mut index = self.index.lock().unwrap();
        *index = (*index + 1) % CLIENT_POOL_SIZE;
        self.pool[*index].clone()
    }
}

// 将 IP 按延迟分组并打乱
fn group_and_shuffle_ips(ip_set: PingDelaySet) -> PingDelaySet {
    if ip_set.is_empty() {
        return ip_set;
    }

    // 1. 按延迟分组
    let mut delay_groups: Vec<Vec<_>> = Vec::new();
    let mut current_group = Vec::new();
    let mut current_delay = ip_set[0].ping_data.delay;

    for ip_data in ip_set {
        if ip_data.ping_data.delay > current_delay + DELAY_GROUP_INTERVAL {
            // 如果延迟差距超过2ms，创建新组
            if !current_group.is_empty() {
                delay_groups.push(current_group);
                current_group = Vec::new();
            }
            current_delay = ip_data.ping_data.delay;
        }
        current_group.push(ip_data);
    }
    
    // 添加最后一组
    if !current_group.is_empty() {
        delay_groups.push(current_group);
    }

    // 2. 打乱每组内的 IP
    let mut rng = rand::thread_rng();
    for group in delay_groups.iter_mut() {
        group.shuffle(&mut rng);
    }

    // 3. 合并所有组
    delay_groups.into_iter().flatten().collect()
}

// 添加一个用于跟踪总带宽的结构
struct TotalBandwidth {
    bandwidth: Arc<Mutex<f64>>,
}

impl TotalBandwidth {
    fn new() -> Self {
        Self {
            bandwidth: Arc::new(Mutex::new(0.0))
        }
    }

    fn add(&self, speed: f64) {
        let mut bandwidth = self.bandwidth.lock().unwrap();
        *bandwidth += speed;
    }

    fn sub(&self, speed: f64) {
        let mut bandwidth = self.bandwidth.lock().unwrap();
        *bandwidth -= speed;
    }

    fn get(&self) -> f64 {
        *self.bandwidth.lock().unwrap()
    }
}

pub async fn test_download_speed(config: &mut Config, ip_set: PingDelaySet) -> Result<DownloadSpeedSet, SpeedTestError> {
    check_download_default(config);

    if config.disable_download {
        return Ok(ip_set);
    }

    if ip_set.is_empty() {
        println!("\n[信息] 延迟测速结果 IP 数量为 0，跳过下载测速。");
        return Ok(ip_set);
    }

    // 在开始下载测速前对 IP 进行分组和打乱
    let ip_set = group_and_shuffle_ips(ip_set);

    let mut test_num = config.test_count;
    if ip_set.len() < test_num as usize || config.min_speed > 0.0 {
        test_num = ip_set.len() as u32;
    }

    println!(
        "开始下载测速（下限：{:.2} MB/s, 数量：{}, 队列：{}）",
        config.min_speed,
        config.test_count,
        test_num
    );

    let bar_padding = " ".repeat(ip_set.len().to_string().len() + 5);
    let bar = Bar::new(config.test_count as u64, &bar_padding, "");

    let semaphore = Arc::new(Semaphore::new(test_num as usize));
    let results = Arc::new(Mutex::new(Vec::new()));
    let fallback_results = Arc::new(Mutex::new(Vec::new()));
    let client_pool = Arc::new(DownloadClient::new());
    let total_bandwidth = Arc::new(TotalBandwidth::new());
    
    let mut handles = Vec::new();
    
    for ip_data in ip_set {
        let permit = semaphore.clone().acquire_owned().await?;
        let config = config.clone();
        let results = results.clone();
        let fallback_results = fallback_results.clone();
        let bar = bar.clone();
        let client_pool = client_pool.clone();
        let total_bandwidth = total_bandwidth.clone();
        
        let handle = tokio::spawn(async move {
            let client = client_pool.get_client();
            let result = download_with_retry(&ip_data.ping_data.ip, &config, &client).await;

            match result {
                Ok(0.0) => {
                    bar.grow(1, &format!("{:.2} MB/s", total_bandwidth.get() / 1024.0 / 1024.0));
                }
                Ok(speed) => {
                    total_bandwidth.add(speed);
                    let mut ip_data = ip_data.clone();
                    ip_data.download_speed = speed;
                    
                    if speed >= config.min_speed * 1024.0 * 1024.0 {
                        let mut results = results.lock().unwrap();
                        results.push(ip_data);
                        bar.grow(1, &format!("{:.2} MB/s", total_bandwidth.get() / 1024.0 / 1024.0));
                    } else {
                        let mut fallback = fallback_results.lock().unwrap();
                        fallback.push(ip_data);
                    }
                    total_bandwidth.sub(speed); // 下载完成后减去这个IP的带宽
                }
                Err(_) => {
                    bar.grow(1, &format!("{:.2} MB/s", total_bandwidth.get() / 1024.0 / 1024.0));
                }
            }
            drop(permit);
        });
        
        handles.push(handle);
    }

    // 并发等待所有任务完成
    futures::future::join_all(handles).await;
    
    // 获取结果
    let mut speed_set = results.lock().unwrap().clone();
    let fallback_set = fallback_results.lock().unwrap().clone();

    // 如果没有满足速度要求的结果，使用 fallback
    if speed_set.is_empty() {
        speed_set = fallback_set;
    }

    bar.done();
    speed_set.sort();
    
    Ok(speed_set)
}

// 下载重试机制
async fn download_with_retry(ip: &IpAddr, config: &Config, client: &Client) -> Result<f64, SpeedTestError> {
    let mut retries = MAX_RETRIES;
    
    while retries > 0 {
        match download_handler(ip, config, client).await {
            Ok(speed) => return Ok(speed),
            Err(_) => {
                retries -= 1;
                if retries > 0 {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    Ok(0.0) // 失败时返回 0
}

fn check_download_default(config: &mut Config) {
    if config.url.is_empty() {
        config.url = DEFAULT_URL.to_string();
    }
    if config.download_time == Duration::ZERO {
        config.download_time = DEFAULT_TIMEOUT;
    }
    if config.test_count == 0 {
        config.test_count = DEFAULT_TEST_NUM;
    }
    if config.min_speed <= 0.0 {
        config.min_speed = DEFAULT_MIN_SPEED;
    }
}

async fn download_handler(_ip: &IpAddr, config: &Config, client: &Client) -> Result<f64, SpeedTestError> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "User-Agent",
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_12_6) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/98.0.4758.80 Safari/537.36"
            .parse()
            .unwrap(),
    );

    let req = reqwest::Request::new(
        reqwest::Method::GET,
        config.url.parse().unwrap()
    );
    
    let mut req = req;
    *req.headers_mut() = headers;

    let response = match client.execute(req).await {
        Ok(resp) => resp,
        Err(_) => return Ok(0.0),
    };

    if response.status() != 200 {
        return Ok(0.0); // 非 200 状态码时返回 0
    }

    let time_start = Instant::now();
    let time_end = time_start + config.download_time;
    let content_length = response.content_length();
    
    let mut content_read = 0u64;
    let time_slice = config.download_time / 100;
    let mut time_counter = 1;
    let mut last_content_read = 0u64;
    
    let mut next_time = time_start + time_slice * time_counter;
    let mut ewma = EWMA::new(0.5);
    let mut last_time_slice = time_start;

    // 创建固定大小的 buffer
    let mut buffer = vec![0u8; BUFFER_SIZE];
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let current_time = Instant::now();
        
        if current_time >= next_time {
            time_counter += 1;
            next_time = time_start + time_slice * time_counter;
            
            // 计算实际时间间隔内的速度
            let duration = current_time.duration_since(last_time_slice);
            if duration > Duration::ZERO {
                let speed = (content_read - last_content_read) as f64 / 
                           (duration.as_secs_f64() / time_slice.as_secs_f64());
                ewma.add(speed);
            }
            
            last_content_read = content_read;
            last_time_slice = current_time;
        }

        if current_time >= time_end {
            break;
        }

        match chunk {
            Ok(data) => {
                // 使用 buffer 来读取数据
                let mut pos = 0;
                while pos < data.len() {
                    let bytes_to_copy = (data.len() - pos).min(buffer.len());
                    buffer[..bytes_to_copy].copy_from_slice(&data[pos..pos + bytes_to_copy]);
                    content_read += bytes_to_copy as u64;
                    pos += bytes_to_copy;

                    // 每次读取后都计算速度
                    let duration = current_time.duration_since(last_time_slice);
                    if duration > Duration::ZERO {
                        let speed = (content_read - last_content_read) as f64 / duration.as_secs_f64();
                        ewma.add(speed);
                        last_content_read = content_read;
                        last_time_slice = current_time;
                    }
                }
                
                // 处理文件下载完成的情况
                if let Some(total_size) = content_length {
                    if content_read >= total_size {
                        // 计算最后一个时间片的速度
                        let duration = current_time.duration_since(last_time_slice);
                        if duration > Duration::ZERO {
                            let speed = (content_read - last_content_read) as f64 / 
                                      (duration.as_secs_f64() / time_slice.as_secs_f64());
                            ewma.add(speed);
                        }
                        break;
                    }
                } else if data.is_empty() {
                    // 文件大小未知且下载完成
                    let duration = current_time.duration_since(last_time_slice);
                    if duration > Duration::ZERO {
                        let speed = (content_read - last_content_read) as f64 / 
                                  (duration.as_secs_f64() / time_slice.as_secs_f64());
                        ewma.add(speed);
                    }
                    break;
                }
            }
            Err(e) => {
                if content_length == Some(content_read) {
                    // 如果已经下载完成，忽略错误
                    break;
                } else if content_length.is_none() && matches!(e.status(), None) {
                    // 文件大小未知且连接已关闭
                    let duration = current_time.duration_since(last_time_slice);
                    if duration > Duration::ZERO {
                        let speed = (content_read - last_content_read) as f64 / 
                                  (duration.as_secs_f64() / time_slice.as_secs_f64());
                        ewma.add(speed);
                    }
                    break;
                }
                return Ok(0.0);
            }
        }
    }

    Ok(ewma.value() / (config.download_time.as_secs_f64() / 120.0))
}

pub async fn build_client(ip: &IpAddr, config: &Config) -> Option<Client> {
    Client::builder()
        .timeout(config.download_time)
        .local_address(Some(*ip))
        .pool_idle_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(5)
        .redirect(redirect::Policy::custom(|attempt| {
            // 限制最多重定向 10 次
            if attempt.previous().len() > 10 {
                attempt.error("too many redirects")
            } else if attempt.previous().first().map(|u| u.as_str()) == Some(DEFAULT_URL) {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .ok()
} 