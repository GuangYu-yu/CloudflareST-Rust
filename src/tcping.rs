use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicUsize, Ordering};
use crate::types::{
    Config, PingDelaySet, CloudflareIPData, PingData
};
use crate::httping::{self, HttpPing};
use crate::progress::Bar;
use crate::ip::{self, IPWithPort};
use tokio::net::TcpStream;
use std::sync::Mutex;
use std::io;
use crate::threadpool::GLOBAL_POOL;
use hyper::{Client, Body};
use hyper::client::HttpConnector;
use hyper_tls::HttpsConnector;

const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(1);

type HandlerResult = Option<PingData>;

#[derive(Debug)]
pub struct Ping {
    m: Arc<Mutex<()>>,
    ips: Vec<IpAddr>,
    csv: PingDelaySet,
    config: Config,
    bar: Bar,
    available_count: Arc<AtomicUsize>,
}

impl Ping {
    fn check_ping_default(&mut self) {
        if self.config.tcp_port <= 0 || self.config.tcp_port >= 65535 {
            self.config.tcp_port = Config::default().tcp_port;
        }
        if self.config.ping_times <= 0 {
            self.config.ping_times = Config::default().ping_times;
        }
    }

    pub async fn run(mut self) -> anyhow::Result<PingDelaySet> {
        self.check_ping_default();
        
        if self.ips.is_empty() {
            return Ok(self.csv);
        }

        println!(
            "开始延迟测速（模式：{}, 端口：{}, 范围：{} ~ {} ms, 丢包：{:.2}）",
            if self.config.httping { "HTTP" } else { "TCP" },
            self.config.tcp_port,
            self.config.min_delay.as_millis(),
            self.config.max_delay.as_millis(),
            self.config.max_loss_rate
        );

        let results = Arc::new(Mutex::new(Vec::new()));
        let mut handles = Vec::new();
        
        for ip in self.ips {
            let config = self.config.clone();
            let bar = self.bar.clone();
            let available_count = self.available_count.clone();
            let m = self.m.clone();
            let results = Arc::clone(&results);
            
            let handle = tokio::spawn(async move {
                // 使用 GLOBAL_POOL 控制并发
                let permit = GLOBAL_POOL.acquire().await;
                
                let result = Self::tcping_handler(ip, &config).await;
                if result.is_some() {
                    available_count.fetch_add(1, Ordering::Relaxed);
                }

                if let Some(ping_data) = result {
                    let _lock = m.lock().unwrap();
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

        // 等待所有任务完成
        futures::future::join_all(handles).await;

        // 获取结果
        let mut results = results.lock().unwrap();
        self.csv = results.drain(..).collect();
        drop(results);

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
            let mut http = HttpConnector::new();
            http.set_local_address(Some(ip));
            let https = HttpsConnector::new_with_connector(http);
            let _client = Client::builder().build::<_, Body>(https);
            
            if !http_ping.check_connection(&_client, &config.url).await? {
                return None;
            }
            httping::http_ping(config, ip).await
        } else {
            Self::check_connection(&ip_with_port, config).await
        }
    }

    pub async fn check_connection(ip_with_port: &IPWithPort, config: &Config) -> Option<PingData> {
        let mut http = HttpConnector::new();
        http.set_local_address(Some(ip_with_port.ip));
        let https = HttpsConnector::new_with_connector(http);
        let _client = Client::builder().build::<_, Body>(https);

        let mut received = 0;
        let mut total_delay = Duration::ZERO;
        let mut delays = Vec::with_capacity(config.ping_times as usize);

        // 收集所有成功的延迟测量
        for _ in 0..config.ping_times {
            if let Some(delay) = tcping(ip_with_port, config).await {
                received += 1;
                total_delay += delay;
                delays.push(delay);
            }
        }

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
            ip_with_port.ip,
            config.ping_times,
            received,
            avg_delay,
        ))
    }
}

pub async fn new_ping(config: Config) -> io::Result<Ping> {
    let ips = ip::load_ip_ranges_concurrent(&config).await?;
    Ok(Ping {
        m: Arc::new(Mutex::new(())),
        ips: ips.clone(),
        csv: Vec::new(),
        config,
        bar: Bar::new(ips.len() as u64, "可用:", ""),
        available_count: Arc::new(AtomicUsize::new(0)),
    })
}

pub async fn tcping(ip_with_port: &IPWithPort, config: &Config) -> Option<Duration> {
    let task_id = rand::random::<usize>();
    GLOBAL_POOL.start_task(task_id);

    let port = ip_with_port.get_port(config.tcp_port);
    let addr = if ip_with_port.ip.is_ipv4() {
        format!("{}:{}", ip_with_port.ip, port)
    } else {
        format!("[{}]:{}", ip_with_port.ip, port)
    };

    let start = Instant::now();
    
    let result = match tokio::time::timeout(
        TCP_CONNECT_TIMEOUT,
        TcpStream::connect(&addr)
    ).await {
        Ok(Ok(stream)) => {
            // 只有成功建立连接才记录进展
            GLOBAL_POOL.record_progress(task_id);
            let duration = start.elapsed();
            drop(stream);
            Some(duration)
        },
        Ok(Err(_)) | Err(_) => None
    };

    GLOBAL_POOL.end_task(task_id);
    result
}

