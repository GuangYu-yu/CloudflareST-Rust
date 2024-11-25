use std::net::IpAddr;
use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::sync::OnceLock;
use regex::Regex;
use reqwest::{Client, redirect, HeaderMap};
use crate::types::Config;

static COLO_REGEX: OnceLock<Regex> = OnceLock::new();

fn get_colo_regex() -> &'static Regex {
    COLO_REGEX.get_or_init(|| Regex::new(r"[A-Z]{3}").unwrap())
}

pub async fn http_ping(config: &Config, ip: IpAddr) -> Option<(u32, Duration)> {
    let client = build_client(ip, config.tcp_port)?;
    
    // 先访问一次获得 HTTP 状态码 及 Cloudflare Colo
    if !check_initial_connection(&client, config).await? {
        return None;
    }

    // 循环测速计算延迟
    let mut success = 0;
    let mut total_delay = Duration::ZERO;

    for i in 0..config.ping_times {
        let mut req = reqwest::Request::new(
            reqwest::Method::HEAD,
            config.url.parse().ok()?
        );
        
        req.headers_mut().insert(
            "User-Agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_12_6) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/98.0.4758.80 Safari/537.36".parse().unwrap()
        );

        if i == config.ping_times - 1 {
            req.headers_mut().insert("Connection", "close".parse().unwrap());
        }

        let start = Instant::now();
        match client.execute(req).await {
            Ok(resp) => {
                success += 1;
                let _ = resp.bytes().await;
                total_delay += start.elapsed();
            }
            Err(_) => continue,
        }
    }

    if success > 0 {
        Some((success, total_delay))
    } else {
        None
    }
}

fn build_client(ip: IpAddr, port: u16) -> Option<Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "User-Agent",
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_12_6) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/98.0.4758.80 Safari/537.36"
            .parse()
            .unwrap(),
    );

    Client::builder()
        .timeout(Duration::from_secs(2))
        .local_address(Some(ip))
        .default_headers(headers)
        .redirect(redirect::Policy::none())
        .connect_timeout(Duration::from_secs(1))
        .build()
        .ok()
}

async fn check_initial_connection(client: &Client, config: &Config) -> Option<bool> {
    let resp = client.head(&config.url).send().await.ok()?;
    
    // 检查状态码
    let status = resp.status().as_u16();
    if config.httping_status_code != 0 
        && (config.httping_status_code < 100 || config.httping_status_code > 599) {
        if status != 200 && status != 301 && status != 302 {
            return None;
        }
    } else if status != config.httping_status_code {
        return None;
    }

    // 只有指定了地区才匹配机场三字码
    if !config.httping_cf_colo.is_empty() {
        let cf_ray = if resp.headers().get("Server").map(|v| v.as_bytes()) == Some(b"cloudflare") {
            resp.headers().get("CF-RAY").and_then(|v| v.to_str().ok())
        } else {
            resp.headers().get("x-amz-cf-pop").and_then(|v| v.to_str().ok())
        };

        if let Some(colo) = cf_ray.and_then(get_colo) {
            if !config.httping_cf_colo.split(',')
                .any(|allowed| allowed.trim().eq_ignore_ascii_case(&colo)) {
                return None;
            }
        } else {
            return None;
        }
    }

    Some(true)
}

fn get_colo(cf_ray: &str) -> Option<String> {
    get_colo_regex().find(cf_ray)
        .map(|m| m.as_str().to_string())
} 