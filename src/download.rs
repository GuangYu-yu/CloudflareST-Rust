use std::net::IpAddr;
use std::time::{Duration, Instant};
use reqwest::{Client, redirect, header::HeaderMap};
use crate::types::{Config, PingDelaySet, DownloadSpeedSet, SpeedTestError, handle_download_error};
use crate::progress::Bar;
use futures::StreamExt;
use ewma::EWMA;
use std::io::Error as IoError;
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;

const BUFFER_SIZE: usize = 1024;
const DEFAULT_URL: &str = "https://cf.xiu2.xyz/url";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_TEST_NUM: u32 = 10;
const DEFAULT_MIN_SPEED: f64 = 0.0;

pub async fn test_download_speed(config: &mut Config, ip_set: PingDelaySet) -> Result<DownloadSpeedSet, SpeedTestError> {
    check_download_default(config);

    if config.disable_download {
        return Ok(ip_set);
    }

    if ip_set.is_empty() {
        println!("\n[信息] 延迟测速结果 IP 数量为 0，跳过下载测速。");
        return Ok(ip_set);
    }

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
    
    let mut handles = Vec::new();
    
    // 使用 stream 处理下载测速
    for ip_data in ip_set {
        let permit = semaphore.clone().acquire_owned().await?;
        let config = config.clone();
        let results = results.clone();
        let fallback_results = fallback_results.clone();
        let bar = bar.clone();
        
        let handle = tokio::spawn(async move {
            let speed = download_handler(&ip_data.ping_data.ip, &config).await;
            if let Ok(speed) = speed {
                let mut ip_data = ip_data.clone();
                ip_data.download_speed = speed;
                
                if speed >= config.min_speed * 1024.0 * 1024.0 {
                    let mut results = results.lock().unwrap();
                    results.push(ip_data);
                    bar.grow(1, "");
                } else {
                    let mut fallback = fallback_results.lock().unwrap();
                    fallback.push(ip_data);
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

async fn download_handler(ip: &IpAddr, config: &Config) -> Result<f64, SpeedTestError> {
    let client = match build_client(ip, config).await {
        Some(c) => c,
        None => return Err(SpeedTestError::DownloadError("Failed to build client".into())),
    };

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

    let response = client.execute(req).await
        .map_err(|e| handle_download_error(IoError::new(
            std::io::ErrorKind::Other,
            e.to_string()
        )))?;

    if response.status() != 200 {
        return Err(SpeedTestError::DownloadError(
            format!("HTTP status: {}", response.status())
        ));
    }

    let time_start = Instant::now();
    let time_end = time_start + config.download_time;
    let content_length = response.content_length().unwrap_or(u64::MAX);
    
    let mut content_read = 0u64;
    let time_slice = config.download_time / 100;
    let mut time_counter = 1;
    let mut last_content_read = 0u64;
    
    let mut next_time = time_start + time_slice * time_counter;
    let mut ewma = EWMA::new(0.5);
    let mut speed_count = 0;

    let mut stream = response.bytes_stream();
    let _buffer = vec![0u8; BUFFER_SIZE];
    while let Some(chunk) = stream.next().await {
        let current_time = Instant::now();
        
        if current_time >= next_time {
            time_counter += 1;
            next_time = time_start + time_slice * time_counter;
            let speed = (content_read - last_content_read) as f64;
            ewma.add(speed);
            speed_count += 1;
            last_content_read = content_read;
        }

        if current_time >= time_end {
            break;
        }

        match chunk {
            Ok(data) => {
                content_read += data.len() as u64;
                if content_length == u64::MAX && data.is_empty() {
                    break;
                }
            }
            Err(e) => {
                return Err(handle_download_error(IoError::new(
                    std::io::ErrorKind::Other,
                    e.to_string()
                )));
            }
        }
    }

    if speed_count > 0 {
        Ok(ewma.value() / (config.download_time.as_secs_f64() / 120.0))
    } else {
        Ok(0.0)
    }
}

pub async fn build_client(ip: &IpAddr, config: &Config) -> Option<Client> {
    Client::builder()
        .timeout(config.download_time)
        .local_address(Some(*ip))
        .redirect(redirect::Policy::custom(|attempt| {
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