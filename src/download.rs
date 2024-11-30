use std::net::IpAddr;
use std::time::{Duration, Instant};
use reqwest::{Client, redirect, header::HeaderMap};
use crate::types::{Config, PingDelaySet, DownloadSpeedSet, SpeedTestError};
use crate::progress::Bar;
use futures::StreamExt;
use ewma::EWMA;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};
use rand::seq::SliceRandom;
use crate::threadpool::GLOBAL_POOL;
use std::collections::HashMap;
use tracing::debug;

const BUFFER_SIZE: usize = 1024;
const DELAY_GROUP_INTERVAL: Duration = Duration::from_millis(2); // 2ms 分组间隔
const MAX_RETRIES: u32 = 3; // 最大重试次数
const CLIENT_POOL_SIZE: usize = 100; // 客户端连接池大小
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(2);
const PROGRESS_MIN_SIZE: u64 = 1024 * 1024; // 1MB

// 客户端连接池
pub struct DownloadClient {
    pool: Arc<Vec<Client>>,
    index: Arc<AtomicUsize>,
    active_connections: Arc<AtomicUsize>,
}

impl DownloadClient {
    pub fn new() -> Self {
        let mut clients = Vec::with_capacity(CLIENT_POOL_SIZE);
        for _ in 0..CLIENT_POOL_SIZE {
            let client = Client::builder()
                .tcp_keepalive(Duration::from_secs(30))
                .tcp_nodelay(true)
                .pool_idle_timeout(Duration::from_secs(30))
                .pool_max_idle_per_host(5)
                .build()
                .unwrap();
            clients.push(client);
        }
        Self {
            pool: Arc::new(clients),
            index: Arc::new(AtomicUsize::new(0)),
            active_connections: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn get_client(&self) -> Client {
        let index = self.index.fetch_add(1, Ordering::Relaxed) % CLIENT_POOL_SIZE;
        self.active_connections.fetch_add(1, Ordering::Relaxed);
        self.pool[index].clone()
    }

    pub fn release_client(&self) {
        self.active_connections.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn get_active_connections(&self) -> usize {
        self.active_connections.load(Ordering::Relaxed)
    }
}

impl Drop for DownloadClient {
    fn drop(&mut self) {
        // 确保所有连接都被正确关闭
        while self.get_active_connections() > 0 {
            self.release_client();
        }
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

pub async fn test_download_speed(config: &mut Config, mut ip_set: PingDelaySet) -> Result<DownloadSpeedSet, SpeedTestError> {
    check_download_default(config);

    if config.disable_download {
        return Ok(ip_set);
    }

    if ip_set.is_empty() {
        println!("\n[信息] 延迟测速结果 IP 数量为 0，跳过下载测速。");
        return Ok(ip_set);
    }

    // 1. 先按延迟排序
    ip_set.sort();

    // 2. 确定测试数量
    let mut test_num = config.test_count;
    if ip_set.len() < test_num as usize || config.min_speed > 0.0 {
        test_num = ip_set.len() as u32;
    }

    // 3. 打印开始信息
    println!(
        "开始下载测速（下限：{:.2} MB/s, 数量：{}, 队列：{}）",
        config.min_speed,
        config.test_count,
        test_num
    );

    // 4. 对要测速的 IP 进行分组和打乱
    let ip_set = group_and_shuffle_ips(ip_set);

    let bar_padding = " ".repeat(ip_set.len().to_string().len() + 5);
    let bar = Bar::new(config.test_count as u64, &bar_padding, "");

    // 使用 HashMap 存储每个 IP 的当前速度
    let current_speeds = Arc::new(Mutex::new(HashMap::new()));
    
    // 5. 创建下载任务
    let results = Arc::new(Mutex::new(Vec::new()));
    let fallback_results = Arc::new(Mutex::new(Vec::new()));
    let client_pool = Arc::new(DownloadClient::new());
    let speed_set = Arc::new(Mutex::new(Vec::new()));
    let ip_set = Arc::new(ip_set);
    
    let mut handles = Vec::new();
    
    for (_index, ip_data) in ip_set.iter().take(test_num.try_into().unwrap()).enumerate() {
        let permit = GLOBAL_POOL.acquire().await;
        
        let config = config.clone();
        let results = results.clone();
        let fallback_results = fallback_results.clone();
        let bar = bar.clone();
        let client_pool = client_pool.clone();
        let ip_data = ip_data.clone();
        let speed_set = speed_set.clone();
        let current_speeds = current_speeds.clone();

        let handle = tokio::spawn(async move {
            let client = client_pool.get_client();
            let ip = ip_data.ping_data.ip;
            
            let result = download_with_retry(&ip, &config, &client, &current_speeds).await;

            match result {
                Ok(speed) => {
                    let mut ip_data_clone = ip_data.clone();
                    ip_data_clone.download_speed = speed;
                    
                    // 更新当前速度表
                    let mut speeds = current_speeds.lock().unwrap();
                    speeds.insert(ip, speed);
                    
                    // 计算总带宽
                    let total_speed: f64 = speeds.values().sum();
                    
                    // 根据速度选择存储位置
                    let results_vec = if speed >= config.min_speed * 1024.0 * 1024.0 {
                        &results
                    } else {
                        &fallback_results
                    };
                    
                    results_vec.lock().unwrap().push(ip_data_clone.clone());
                    speed_set.lock().unwrap().push(ip_data_clone);
                    bar.grow(1, &format!("{:.2} MB/s", total_speed / 1024.0 / 1024.0));
                }
                Err(_) => {
                    // 计算当前总带宽
                    let speeds = current_speeds.lock().unwrap();
                    let total_speed: f64 = speeds.values().sum();
                    bar.grow(1, &format!("{:.2} MB/s", total_speed / 1024.0 / 1024.0));
                }
            }

            // 测试完成后移除该 IP 的速度记录
            current_speeds.lock().unwrap().remove(&ip);
            drop(permit);
        });
        handles.push(handle);
    }

    // 等待所有任务完成
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
    
    // 如果不是禁用下载测速，则按下载速度排序
    if !config.disable_download {
        speed_set.sort_by(|a, b| b.download_speed.partial_cmp(&a.download_speed).unwrap());
    }
    
    Ok(speed_set)
}

fn check_download_default(config: &mut Config) {
    let default_config = Config::default();
    if config.url.is_empty() {
        config.url = default_config.url;
    }
    if config.download_time == Duration::ZERO {
        config.download_time = default_config.download_time;
    }
    if config.test_count == 0 {
        config.test_count = default_config.test_count;
    }
    if config.min_speed <= 0.0 {
        config.min_speed = default_config.min_speed;
    }
}

async fn download_with_retry(
    ip: &IpAddr,
    config: &Config,
    client: &Client,
    current_speeds: &Arc<Mutex<HashMap<IpAddr, f64>>>
) -> Result<f64, SpeedTestError> {
    let mut retries = MAX_RETRIES;
    let mut last_error = None;
    
    while retries > 0 {
        match download_handler(ip, config, client, current_speeds).await {
            Ok(speed) => {
                if speed > 0.0 {
                    return Ok(speed);
                }
            }
            Err(e) => {
                last_error = Some(e);
                retries -= 1;
                if retries > 0 {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    if let Some(e) = last_error {
        Err(e)
    } else {
        Ok(0.0)
    }
}

async fn download_handler(
    ip: &IpAddr,
    config: &Config,
    client: &Client,
    current_speeds: &Arc<Mutex<HashMap<IpAddr, f64>>>
) -> Result<f64, SpeedTestError> {
    let task_id = rand::random::<usize>();
    GLOBAL_POOL.start_task(task_id);

    debug!("开始下载测速 IP: {}", ip);
    
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
        Ok(resp) => {
            debug!("收到响应: 状态码={}", resp.status());
            resp
        },
        Err(e) => {
            debug!("请求失败: {}", e);
            return Ok(0.0);
        }
    };

    if response.status() != 200 {
        debug!("非200状态码: {}", response.status());
        return Ok(0.0);
    }

    let time_start = Instant::now();
    let time_end = time_start + config.download_time;
    let content_length = response.content_length();
    
    if let Some(size) = content_length {
        debug!("开始下载: 文件大小={} bytes", size);
    }

    let mut content_read = 0u64;
    let time_slice = config.download_time / 100;
    let mut time_counter = 1;
    let mut last_content_read = 0u64;
    
    let mut next_time = time_start + time_slice * time_counter;
    let mut ewma = EWMA::new(0.5);
    let mut last_time_slice = time_start;

    let mut buffer = vec![0u8; BUFFER_SIZE];
    let mut stream = response.bytes_stream();

    let mut speed_samples = Vec::new();
    let sample_interval = config.download_time / 10;
    let mut next_sample = time_start + sample_interval;

    let mut last_transfer = Instant::now();

    while let Some(chunk) = stream.next().await {
        let current_time = Instant::now();
        
        // 每次收到数据块就记录进展
        GLOBAL_POOL.record_progress(task_id);
        
        let chunk_size = chunk.as_ref().map(|c| c.len()).unwrap_or(0);
        debug!("接收数据块: {} bytes, 总计: {} bytes", chunk_size, content_read + chunk_size as u64);
        
        // 检查总下载时间
        if current_time >= time_end {
            break;
        }
        
        // 检查传输超时
        if current_time.duration_since(last_transfer) > TRANSFER_TIMEOUT {
            return Ok(0.0);  // 超时时返回 0 速度
        }

        // 更新传输时间
        last_transfer = current_time;
        
        match chunk {
            Ok(data) => {
                // 使用 buffer 来读取数据
                let mut pos = 0;
                while pos < data.len() {
                    let bytes_to_copy = (data.len() - pos).min(buffer.len());
                    buffer[..bytes_to_copy].copy_from_slice(&data[pos..pos + bytes_to_copy]);
                    content_read += bytes_to_copy as u64;
                    pos += bytes_to_copy;

                    // 检查是否需要采样
                    let current_time = Instant::now();
                    if current_time >= next_sample {
                        let duration = current_time.duration_since(last_time_slice);
                        if duration > Duration::ZERO {
                            let speed = (content_read - last_content_read) as f64 / duration.as_secs_f64();
                            speed_samples.push(speed);
                            ewma.add(speed);
                            current_speeds.lock().unwrap().insert(*ip, ewma.value());
                            
                            last_content_read = content_read;
                            last_time_slice = current_time;
                            next_sample = current_time + sample_interval;
                        }
                    }
                }

                // 处理下载完成的情况
                if let Some(total_size) = content_length {
                    if content_read >= total_size {
                        let final_speed = calculate_final_speed(&speed_samples, ewma.value());
                        return Ok(final_speed);
                    }
                }
            }
            Err(e) => {
                if content_read == 0 {
                    return Err(SpeedTestError::RequestError(e));
                }
                // 如果已经有一些数据，计算部分速度
                let final_speed = calculate_final_speed(&speed_samples, ewma.value());
                return Ok(final_speed);
            }
        }

        // 定期采样速度
        if current_time >= next_time && content_read >= PROGRESS_MIN_SIZE {
            time_counter += 1;
            next_time = time_start + time_slice * time_counter;
            
            let duration = current_time.duration_since(last_time_slice);
            if duration > Duration::ZERO {
                let speed = (content_read - last_content_read) as f64 / duration.as_secs_f64();
                speed_samples.push(speed);
                ewma.add(speed);
                
                // 更新当前速度
                current_speeds.lock().unwrap().insert(*ip, ewma.value());
                
                last_content_read = content_read;
                last_time_slice = current_time;
            }
        }
    }

    debug!("下载完成: IP={}, 总下载量={} bytes", ip, content_read);
    GLOBAL_POOL.end_task(task_id);

    let final_speed = calculate_final_speed(&speed_samples, ewma.value());
    debug!("最终速度: {:.2} MB/s", final_speed / 1024.0 / 1024.0);
    Ok(final_speed)
}

fn calculate_final_speed(samples: &[f64], ewma_value: f64) -> f64 {
    if samples.is_empty() {
        return ewma_value;
    }

    // 去除最高和最低的 10% 样本
    let mut sorted_samples = samples.to_vec();
    sorted_samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    
    let trim_count = (samples.len() as f64 * 0.1) as usize;
    let valid_samples = &sorted_samples[trim_count..sorted_samples.len() - trim_count];
    
    if valid_samples.is_empty() {
        return ewma_value;
    }

    // 计算平均速度
    let avg_speed = valid_samples.iter().sum::<f64>() / valid_samples.len() as f64;
    
    // 使用 EWMA 值和平均值的加权平均作为最终速度
    ewma_value * 0.7 + avg_speed * 0.3
}

pub async fn build_client(ip: &IpAddr, config: &Config) -> Option<Client> {
    Client::builder()
        .timeout(config.download_time)
        .connect_timeout(CONNECT_TIMEOUT)
        .pool_idle_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(5)
        .local_address(Some(*ip))
        .tcp_keepalive(Duration::from_secs(30))
        .tcp_nodelay(true)
        .redirect(redirect::Policy::custom(|attempt| {
            // 限制最多重定向 10 次
            if attempt.previous().len() > 10 {
                attempt.error("too many redirects")
            } else if attempt.previous().first().map(|u| u.as_str()) == Some(&Config::default().url) {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .ok()
} 