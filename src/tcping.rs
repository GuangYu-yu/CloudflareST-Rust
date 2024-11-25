use std::net::{IpAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::io;
use tokio::sync::Semaphore;
use crate::types::{
    Config, PingDelaySet, CloudflareIPData, PingData
};
use crate::httping::{self, HttpPing};
use crate::progress::Bar;
use crate::ip;
use crate::download::build_client;

const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_ROUTINES: u32 = 1000;
const DEFAULT_ROUTINES: u32 = 200;
const DEFAULT_PORT: u16 = 443;
const DEFAULT_PING_TIMES: u32 = 4;

pub struct Ping {
    pool: Arc<Semaphore>,
    m: Arc<Mutex<()>>,
    ips: Vec<IpAddr>,
    csv: PingDelaySet,
    config: Config,
    bar: Bar,
}

impl Ping {
    fn check_ping_default(&mut self) {
        if self.config.routines <= 0 {
            self.config.routines = DEFAULT_ROUTINES;
        }
        if self.config.routines > MAX_ROUTINES {
            self.config.routines = MAX_ROUTINES;
        }
        if self.config.tcp_port <= 0 || self.config.tcp_port >= 65535 {
            self.config.tcp_port = DEFAULT_PORT;
        }
        if self.config.ping_times <= 0 {
            self.config.ping_times = DEFAULT_PING_TIMES;
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

        let mut handles = Vec::with_capacity(self.ips.len());
        
        for ip in self.ips {
            let permit = self.pool.clone().acquire_owned().await?;
            let config = self.config.clone();
            let bar = self.bar.clone();
            
            let handle = tokio::spawn(async move {
                let result = Self::tcping_handler(ip, &config).await;
                drop(permit);
                (result, bar)
            });
            
            handles.push(handle);
        }

        use futures::StreamExt;
        let results = futures::stream::iter(handles)
            .buffer_unordered(self.config.routines as usize)
            .collect::<Vec<_>>()
            .await;

        for handle in results {
            if let Ok((result, bar)) = handle {
                if let Some(data) = result {
                    let _lock = self.m.lock().unwrap();
                    let mut ip_data = CloudflareIPData::new(data);
                    ip_data.config = self.config.clone();
                    self.csv.push(ip_data);
                }
                bar.grow(1, &format!("{}", self.csv.len()));
            }
        }

        self.bar.done();
        self.csv.sort();
        Ok(self.csv)
    }

    pub async fn tcping_handler(ip: IpAddr, config: &Config) -> Option<PingData> {
        let (recv, total_delay) = if config.httping {
            let http_ping = HttpPing::new(config.clone(), Some(&config.httping_cf_colo));
            let client = build_client(&ip, config).await?;
            if !http_ping.check_connection(&client, &config.url).await? {
                return None;
            }
            httping::http_ping(config, ip).await?
        } else {
            Self::check_connection(ip, config).await
        };

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

pub async fn new_ping(config: Config) -> io::Result<Ping> {
    let ips = ip::load_ip_ranges_concurrent(&config).await?;
    Ok(Ping {
        pool: Arc::new(Semaphore::new(config.routines as usize)),
        m: Arc::new(Mutex::new(())),
        ips: ips.clone(),
        csv: Vec::new(),
        config,
        bar: Bar::new(ips.len() as u64, "可用:", ""),
    })
}

