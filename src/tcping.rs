use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use std::sync::atomic::{AtomicUsize, Ordering};
use futures::StreamExt;
use crate::types::{
    Config, PingDelaySet, CloudflareIPData, PingData, SpeedTestError
};
use crate::httping::{self, HttpPing};
use crate::progress::Bar;
use crate::ip::{self, IPWithPort};
use crate::download::build_client;
use tokio::net::TcpStream;
use std::sync::Mutex;
use std::io;

const fn default_routines() -> u32 { 200 }
const fn default_port() -> u16 { 443 }
const fn default_ping_times() -> u32 { 4 }
const fn max_routines() -> u32 { 1000 }
const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(1);

type HandlerResult = Option<PingData>;

#[derive(Debug)]
pub struct Ping {
    pool: Arc<Semaphore>,
    m: Arc<Mutex<()>>,
    ips: Vec<IpAddr>,
    csv: PingDelaySet,
    config: Config,
    bar: Bar,
    available_count: Arc<AtomicUsize>,
}

impl Ping {
    fn check_ping_default(&mut self) {
        if self.config.routines <= 0 {
            self.config.routines = default_routines();
        }
        if self.config.routines > max_routines() {
            self.config.routines = max_routines();
        }
        if self.config.tcp_port <= 0 || self.config.tcp_port >= 65535 {
            self.config.tcp_port = default_port();
        }
        if self.config.ping_times <= 0 {
            self.config.ping_times = default_ping_times();
        }
    }

    pub async fn run(mut self) -> anyhow::Result<PingDelaySet> {
        self.check_ping_default();
        
        if self.ips.is_empty() {
            return Ok(self.csv);
        }

        let mode = if self.config.httping { "HTTP" } else { "TCP" };
        println!(
            "开始延迟测速（模式：{}, 端口：{}, 范围：{} ~ {} ms, 丢包：{:.2}）",
            mode,
            self.config.tcp_port,
            self.config.min_delay.as_millis(),
            self.config.max_delay.as_millis(),
            self.config.max_loss_rate
        );

        let results = futures::stream::iter(self.ips)
            .map(|ip| {
                let config = self.config.clone();
                let bar = self.bar.clone();
                let available_count = self.available_count.clone();
                let permit = self.pool.clone().acquire_owned();
                
                async move {
                    match permit.await {
                        Ok(_permit) => {
                            let result = Self::tcping_handler(ip, &config).await;
                            if result.is_some() {
                                available_count.fetch_add(1, Ordering::Relaxed);
                            }
                            Ok((result, bar, available_count.load(Ordering::Relaxed)))
                        }
                        Err(e) => Err(SpeedTestError::from(e))
                    }
                }
            })
            .buffer_unordered(self.config.routines as usize)
            .collect::<Vec<_>>()
            .await;

        for result in results {
            if let Ok((data, bar, _count)) = result {
                if let Some(ping_data) = data {
                    let _lock = self.m.lock().unwrap();
                    let mut ip_data = CloudflareIPData::new(ping_data);
                    ip_data.config = self.config.clone();
                    self.csv.push(ip_data);
                    let now_able = self.csv.len();
                    bar.grow(1, &now_able.to_string());
                } else {
                    bar.grow(1, &self.csv.len().to_string());
                }
            }
        }

        self.bar.done();
        self.csv.sort();
        Ok(self.csv)
    }

    pub async fn tcping_handler(ip: IpAddr, config: &Config) -> HandlerResult {
        let ip_with_port = IPWithPort { 
            ip, 
            port: None 
        };

        if config.httping {
            let http_ping = HttpPing::new(config.clone(), Some(&config.httping_cf_colo));
            let client = build_client(&ip, config).await?;
            if !http_ping.check_connection(&client, &config.url).await? {
                return None;
            }
            httping::http_ping(config, ip).await
        } else {
            Self::check_connection(&ip_with_port, config).await
        }
    }

    async fn check_connection(ip_with_port: &IPWithPort, config: &Config) -> Option<PingData> {
        let mut received = 0;
        let mut total_delay = Duration::ZERO;

        for _ in 0..config.ping_times {
            if let Some(delay) = tcping(ip_with_port, config).await {
                received += 1;
                total_delay += delay;
            }
        }

        if received == 0 {
            return None;
        }

        Some(PingData::new(
            ip_with_port.ip,
            config.ping_times,
            received,
            total_delay / received as u32,
        ))
    }
}

pub async fn new_ping(config: Config) -> io::Result<Ping> {
    let ips = ip::load_ip_ranges_concurrent(&config).await?;
    Ok(Ping {
        pool: Arc::new(Semaphore::new(config.routines as usize)),
        m: Arc::new(Mutex::new(())),
        ips: ips.clone(),
        csv: Vec::new(),
        config,
        bar: Bar::new(ips.len() as u64, "可用:", ""),
        available_count: Arc::new(AtomicUsize::new(0)),
    })
}

pub async fn tcping(ip_with_port: &IPWithPort, config: &Config) -> Option<Duration> {
    let port = ip_with_port.get_port(config.tcp_port);
    let addr = if ip_with_port.ip.is_ipv4() {
        format!("{}:{}", ip_with_port.ip, port)
    } else {
        format!("[{}]:{}", ip_with_port.ip, port)
    };

    let start = Instant::now();
    match tokio::time::timeout(
        TCP_CONNECT_TIMEOUT,
        TcpStream::connect(&addr)
    ).await {
        Ok(Ok(_)) => Some(start.elapsed()),
        _ => None
    }
}

