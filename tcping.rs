use std::net::{IpAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use crate::types::{Config, PingDelaySet, CloudflareIPData, PingData};
use crate::httping;
use crate::progress::Bar;
use crate::ip;

const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_ROUTINES: u32 = 1000;
const DEFAULT_ROUTINES: u32 = 200;
const DEFAULT_PORT: u16 = 443;
const DEFAULT_PING_TIMES: u32 = 4;

pub struct Ping<'a> {
    wg: Arc<Semaphore>,
    m: Arc<Mutex<()>>,
    ips: Vec<IpAddr>,
    csv: PingDelaySet,
    config: &'a Config,
    bar: Bar,
}

impl<'a> Ping<'a> {
    fn check_ping_default(&self) -> (u32, u16, u32) {
        let mut routines = self.config.routines;
        let mut port = self.config.tcp_port;
        let mut ping_times = self.config.ping_times;

        if routines <= 0 {
            routines = DEFAULT_ROUTINES;
        }
        if routines > MAX_ROUTINES {
            routines = MAX_ROUTINES;
        }
        if port <= 0 || port >= 65535 {
            port = DEFAULT_PORT;
        }
        if ping_times <= 0 {
            ping_times = DEFAULT_PING_TIMES;
        }

        (routines, port, ping_times)
    }

    pub async fn run(mut self) -> anyhow::Result<PingDelaySet> {
        if self.ips.is_empty() {
            return Ok(self.csv);
        }

        if self.config.httping {
            println!(
                "开始延迟测速（模式：HTTP, 端口：{}, 范围：{} ~ {} ms, 丢包：{:.2}）",
                self.config.tcp_port,
                self.config.min_delay.as_millis(),
                self.config.max_delay.as_millis(),
                self.config.max_loss_rate
            );
        } else {
            println!(
                "开始延迟测速（模式：TCP, 端口：{}, 范围：{} ~ {} ms, 丢包：{:.2}）",
                self.config.tcp_port,
                self.config.min_delay.as_millis(),
                self.config.max_delay.as_millis(),
                self.config.max_loss_rate
            );
        }

        let mut handles = vec![];
        for ip in self.ips {
            let permit = self.wg.acquire().await?;
            let m = Arc::clone(&self.m);
            let config = self.config;
            let bar = self.bar.clone();
            
            let handle = tokio::spawn(async move {
                let result = Self::tcping_handler(ip, config).await;
                drop(permit);
                (result, m, bar)
            });
            handles.push(handle);
        }

        for handle in handles {
            if let Ok((result, m, bar)) = handle.await {
                if let Some(data) = result {
                    let _lock = m.lock().unwrap();
                    self.csv.push(CloudflareIPData::new(data));
                }
                bar.grow(1, &format!("{}", self.csv.len()));
            }
        }

        self.bar.done();
        self.csv.sort();
        Ok(self.csv)
    }

    async fn tcping_handler(ip: IpAddr, config: &Config) -> Option<PingData> {
        let (recv, total_delay) = Self::check_connection(ip, config).await;
        if recv == 0 {
            return None;
        }

        Some(PingData::new(
            ip,
            config.ping_times,
            recv,
            total_delay / recv as u32,
        ))
    }

    async fn check_connection(ip: IpAddr, config: &Config) -> (u32, Duration) {
        if config.httping {
            if let Some((received, delay)) = httping::http_ping(config, ip).await {
                return (received, delay);
            }
            return (0, Duration::ZERO);
        }

        let mut received = 0;
        let mut total_delay = Duration::ZERO;

        for _ in 0..config.ping_times {
            if let Some(delay) = Self::tcping(ip, config.tcp_port).await {
                received += 1;
                total_delay += delay;
            }
        }

        (received, total_delay)
    }

    async fn tcping(ip: IpAddr, port: u16) -> Option<Duration> {
        let addr = format!("{}:{}", ip, port);
        let start = Instant::now();
        
        match TcpStream::connect_timeout(&addr.parse().ok()?, TCP_CONNECT_TIMEOUT) {
            Ok(_) => Some(start.elapsed()),
            Err(_) => None,
        }
    }
}

pub fn new_ping(config: &Config) -> Ping {
    let ips = ip::load_ip_ranges();
    Ping {
        wg: Arc::new(Semaphore::new(config.routines as usize)),
        m: Arc::new(Mutex::new(())),
        ips,
        csv: Vec::new(),
        config,
        bar: Bar::new(ips.len() as u64, "可用:", ""),
    }
} 