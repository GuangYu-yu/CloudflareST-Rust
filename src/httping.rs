use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use hyper::{Client, Request, Body, Method};
use hyper::client::HttpConnector;
use hyper_tls::HttpsConnector;
use hyper::header::HeaderMap;
use hyper::body::HttpBody;
use regex::Regex;
use lazy_static::lazy_static;
use crate::types::{Config, PingData, PingDelaySet};
use crate::progress::Bar;
use futures::StreamExt;
use tokio::io::AsyncWriteExt;
use std::collections::HashMap;
use std::sync::Mutex;
use crate::threadpool::GLOBAL_POOL;
use crate::types::CloudflareIPData;

lazy_static! {
    static ref COLO_REGEX: Regex = Regex::new(r"[A-Z]{3}").unwrap();
}

const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_12_6) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/98.0.4758.80 Safari/537.36";

#[derive(Clone)]
pub struct HttpPing {
    config: Config,
    colo_filter: Option<String>,
    colo_map: Option<Arc<HashMap<String, String>>>,
}

impl HttpPing {
    pub fn new(config: Config, colo_filter: Option<&str>) -> Self {
        let colo_map = colo_filter.map(|filter| {
            let map = filter
                .split(',')
                .map(|s| {
                    let s = s.trim().to_uppercase();
                    (s.clone(), s)
                })
                .collect();
            Arc::new(map)
        });

        Self {
            config,
            colo_filter: colo_filter.map(|s| s.to_string()),
            colo_map,
        }
    }

    pub async fn check_connection(&self, client: &Client<HttpsConnector<HttpConnector>>, url: &str) -> Option<bool> {
        let request = Request::builder()
            .method("HEAD")
            .uri(url)
            .header("Accept", "*/*")
            .header("User-Agent", USER_AGENT)
            .body(Body::empty())
            .ok()?;

        let response = match client.request(request).await {
            Ok(r) => r,
            Err(_) => return None,
        };

        let status = response.status().as_u16();
        let headers = response.headers().clone();

        // 显式处理响应体
        let mut body = response.into_body();
        let mut sink = tokio::io::sink();
        while let Some(chunk) = body.next().await {
            if chunk.is_err() || sink.write(&chunk.unwrap()).await.is_err() {
                return None;
            }
        }

        let valid_status = if self.config.httping_status_code == 0 
            || self.config.httping_status_code < 100 
            || self.config.httping_status_code > 599 
        {
            let default_config = Config::default();
            status == default_config.httping_status_code
        } else {
            status == self.config.httping_status_code
        };

        if !valid_status {
            return Some(false);
        }

        if !self.config.httping_cf_colo.is_empty() {
            if let Some(colo) = self.get_colo(&headers) {
                if !self.match_colo(&colo) {
                    return Some(false);
                }
            } else {
                return Some(false);
            }
        }

        Some(true)
    }

    pub fn get_colo(&self, headers: &HeaderMap) -> Option<String> {
        let cf_ray = if headers.get("Server").map_or(false, |v| v.to_str().ok() == Some("cloudflare")) {
            headers.get("CF-RAY").and_then(|v| v.to_str().ok())
        } else {
            headers.get("x-amz-cf-pop").and_then(|v| v.to_str().ok())
        };

        cf_ray.and_then(|ray| self.extract_colo(ray))
    }

    fn extract_colo(&self, cf_ray: &str) -> Option<String> {
        COLO_REGEX.find(cf_ray).map(|m| m.as_str().to_string())
    }

    pub fn match_colo(&self, colo: &str) -> bool {
        if let Some(filter) = &self.colo_filter {
            filter.contains(colo)
        } else if let Some(map) = &self.colo_map {
            map.contains_key(colo)
        } else {
            true
        }
    }

    async fn build_client(&self, ip: IpAddr) -> Client<HttpsConnector<HttpConnector>> {
        let mut http = HttpConnector::new();
        http.set_local_address(Some(ip));
        let https = HttpsConnector::new_with_connector(http);
        Client::builder().build::<_, Body>(https)
    }

    pub async fn http_ping(&self, config: &Config, ip: IpAddr) -> Option<PingData> {
        let task_id = rand::random::<usize>();
        GLOBAL_POOL.start_task(task_id);

        let client = self.build_client(ip).await;

        // 检查连接时也记录进展
        if let Some(true) = self.check_connection(&client, &config.url).await {
            GLOBAL_POOL.record_progress(task_id);
        }

        let mut received = 0;
        let mut total_delay = Duration::ZERO;
        let mut delays = Vec::with_capacity(config.ping_times as usize);

        for i in 0..config.ping_times {
            let start = Instant::now();
            let request = Request::builder()
                .method(Method::HEAD)
                .uri(&config.url)
                .header("Accept", "*/*")
                .header("User-Agent", USER_AGENT)
                .header("Connection", if i == config.ping_times - 1 { "close" } else { "keep-alive" })
                .body(Body::empty())
                .ok()?;

            match client.request(request).await {
                Ok(response) => {
                    let status = response.status();
                    if !status.is_success() && !status.is_redirection() {
                        continue;
                    }

                    let mut body = response.into_body();
                    let mut sink = tokio::io::sink();
                    let mut success = true;
                    
                    while let Some(chunk) = body.data().await {
                        if chunk.is_err() || sink.write(&chunk.unwrap()).await.is_err() {
                            success = false;
                            break;
                        }
                    }

                    if success {
                        let delay = start.elapsed();
                        received += 1;
                        total_delay += delay;
                        delays.push(delay);
                        // 每次成功的请求都记录进展
                        GLOBAL_POOL.record_progress(task_id);
                    }
                }
                Err(_) => continue,
            }
        }

        GLOBAL_POOL.end_task(task_id);

        if received == 0 {
            return None;
        }

        // 计算平均延迟，保持精确度
        let avg_delay = if delays.len() > 2 {
            // 去掉最高和最低值后取平均
            delays.sort();
            let valid_delays = &delays[1..delays.len()-1];
            let sum: Duration = valid_delays.iter().sum();
            sum / valid_delays.len() as u32
        } else {
            total_delay / received as u32
        };

        Some(PingData::new(
            ip,
            config.ping_times,
            received,
            avg_delay,
        ))
    }

    pub async fn http_ping_all(&self, config: &Config, ip_list: &[IpAddr]) -> PingDelaySet {
        let results = Arc::new(Mutex::new(Vec::new()));
        let bar = Bar::new(ip_list.len() as u64, "可用:", "");
        let mut handles = Vec::new();

        for &ip in ip_list {
            let permit = GLOBAL_POOL.acquire().await;
            let config = config.clone();
            let results = Arc::clone(&results);
            let http_ping = self.clone();
            let bar = bar.clone();

            let handle = tokio::spawn(async move {
                if let Some(ping_data) = http_ping.http_ping(&config, ip).await {
                    let mut ip_data = CloudflareIPData::new(ping_data);
                    ip_data.config = config;
                    let mut results = results.lock().unwrap();
                    results.push(ip_data);
                    let now_able = results.len();
                    bar.grow(1, &now_able.to_string());
                } else {
                    let results = results.lock().unwrap();
                    bar.grow(1, &results.len().to_string());
                }
                drop(permit);
            });

            handles.push(handle);
        }

        futures::future::join_all(handles).await;
        
        let mut results = results.lock().unwrap();
        let mut ping_data = results.drain(..).collect::<Vec<_>>();
        drop(results);
        
        bar.done();
        ping_data.sort();
        ping_data
    }
}

pub async fn http_ping(config: &Config, ip: IpAddr) -> Option<PingData> {
    let http_ping = HttpPing::new(config.clone(), Some(&config.httping_cf_colo));
    http_ping.http_ping(config, ip).await
}
