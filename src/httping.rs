use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use reqwest::header::HeaderMap;
use regex::Regex;
use lazy_static::lazy_static;
use crate::types::{Config, PingData};
use crate::download::build_client;
use futures::StreamExt;
use tokio::io::AsyncWriteExt;
use std::collections::HashMap;

lazy_static! {
    static ref COLO_REGEX: Regex = Regex::new(r"[A-Z]{3}").unwrap();
}

const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_12_6) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/98.0.4758.80 Safari/537.36";

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

    pub async fn check_connection(&self, client: &reqwest::Client, url: &str) -> Option<bool> {
        let resp = match client
            .head(url)
            .header("User-Agent", USER_AGENT)
            .send()
            .await {
                Ok(r) => r,
                Err(_) => return None,
            };

        let status = resp.status().as_u16();
        let headers = resp.headers().clone();

        // 显式处理响应体
        let mut body = resp.bytes_stream();
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
            status == 200 || status == 301 || status == 302
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
        let server = headers.get("Server")?;
        let cf_ray = if server.as_bytes() == b"cloudflare" {
            headers.get("CF-RAY")?.to_str().ok()?
        } else if server.as_bytes() == b"cloudfront" {
            headers.get("x-amz-cf-pop")?.to_str().ok()?
        } else {
            return None;
        };

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

    pub async fn http_ping(&self, config: &Config, ip: IpAddr) -> Option<PingData> {
        let client = build_client(&ip, config).await?;

        if let Some(false) = self.check_connection(&client, &config.url).await {
            return None;
        }

        let mut received = 0;
        let mut total_delay = Duration::ZERO;

        for i in 0..config.ping_times {
            let start = Instant::now();
            let result = client
                .head(&config.url)
                .header("User-Agent", USER_AGENT)
                .header(
                    "Connection",
                    if i == config.ping_times - 1 { "close" } else { "keep-alive" },
                )
                .send()
                .await;

            match result {
                Ok(resp) => {
                    let status = resp.status();
                    if !status.is_success() && !status.is_redirection() {
                        continue;
                    }

                    // 显式处理响应体
                    let mut body = resp.bytes_stream();
                    let mut sink = tokio::io::sink();
                    let mut success = true;
                    
                    while let Some(chunk) = body.next().await {
                        if chunk.is_err() || sink.write(&chunk.unwrap()).await.is_err() {
                            success = false;
                            break;
                        }
                    }

                    if success {
                        received += 1;
                        total_delay += start.elapsed();
                    }
                }
                Err(_) => continue,
            }
        }

        if received == 0 {
            return None;
        }

        Some(PingData::new(
            ip,
            config.ping_times,
            received,
            total_delay / received as u32,
        ))
    }
}

pub async fn http_ping(config: &Config, ip: IpAddr) -> Option<PingData> {
    let http_ping = HttpPing::new(config.clone(), Some(&config.httping_cf_colo));
    http_ping.http_ping(config, ip).await
}
