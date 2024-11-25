use std::net::IpAddr;
use std::time::{Duration, Instant};
use reqwest::{Client, redirect, header::HeaderMap};
use tokio::io::AsyncReadExt;
use crate::types::{Config, PingDelaySet, DownloadSpeedSet};
use crate::progress::Bar;
use ewma::MovingAverage;
use futures::StreamExt;

const BUFFER_SIZE: usize = 1024;
const DEFAULT_URL: &str = "https://cf.xiu2.xyz/url";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_DISABLE: bool = false;
const DEFAULT_TEST_NUM: u32 = 10;
const DEFAULT_MIN_SPEED: f64 = 0.0;

pub async fn test_download_speed(config: &Config, ip_set: PingDelaySet) -> anyhow::Result<DownloadSpeedSet> {
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

    let bar = Bar::new(config.test_count as u64, "", "");
    let mut speed_set = Vec::new();

    for (i, mut ip_data) in ip_set.into_iter().take(test_num as usize).enumerate() {
        let speed = download_handler(&ip_data.ping_data.ip, config).await;
        ip_data.download_speed = speed;

        if speed >= config.min_speed * 1024.0 * 1024.0 {
            bar.grow(1, "");
            speed_set.push(ip_data);
            if speed_set.len() == config.test_count as usize {
                break;
            }
        }
    }

    bar.done();

    if speed_set.is_empty() {
        speed_set = ip_set;
    }

    speed_set.sort();
    Ok(speed_set)
}

fn check_download_default(config: &Config) {
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

async fn download_handler(ip: &IpAddr, config: &Config) -> f64 {
    let client = match build_client(ip, config).await {
        Some(c) => c,
        None => return 0.0,
    };

    let mut headers = HeaderMap::new();
    headers.insert(
        "User-Agent",
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_12_6) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/98.0.4758.80 Safari/537.36"
            .parse()
            .unwrap(),
    );

    let req = match reqwest::Request::new(reqwest::Method::GET, config.url.parse().unwrap()) {
        Ok(mut req) => {
            *req.headers_mut() = headers;
            req
        }
        Err(_) => return 0.0,
    };

    let response = match client.execute(req).await {
        Ok(resp) => {
            if resp.status() != 200 {
                return 0.0;
            }
            resp
        }
        Err(_) => return 0.0,
    };

    let time_start = Instant::now();
    let time_end = time_start + config.download_time;
    let content_length = response.content_length().unwrap_or(-1);
    let mut buffer = vec![0u8; BUFFER_SIZE];

    let mut content_read = 0i64;
    let time_slice = config.download_time / 100;
    let mut time_counter = 1;
    let mut last_content_read = 0i64;

    let mut next_time = time_start + time_slice * time_counter;
    let mut e = ewma::MovingAverage::new(1.0);

    let mut stream = response.bytes_stream();
    while content_length != content_read {
        let current_time = Instant::now();
        
        if current_time >= next_time {
            time_counter += 1;
            next_time = time_start + time_slice * time_counter;
            e.add(content_read - last_content_read);
            last_content_read = content_read;
        }

        if current_time >= time_end {
            break;
        }

        match stream.read(&mut buffer).await {
            Ok(n) => {
                if n == 0 {
                    if content_length == -1 {
                        break;
                    }
                    let last_time_slice = time_start + time_slice * (time_counter - 1);
                    e.add((content_read - last_content_read) as f64 
                        / (current_time - last_time_slice).as_secs_f64() 
                        * time_slice.as_secs_f64());
                }
                content_read += n as i64;
            }
            Err(_) => break,
        }
    }

    e.value() / (config.download_time.as_secs_f64() / 120.0)
}

async fn build_client(ip: &IpAddr, config: &Config) -> Option<Client> {
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