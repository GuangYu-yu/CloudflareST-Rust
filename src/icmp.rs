use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::io;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use surge_ping::{Client, Config, PingIdentifier, PingSequence, ICMP};
use rand::random;
use crate::pool::execute_with_rate_limit;

use crate::args::Args;
use crate::common::{self, PingData, PingDelaySet, HandlerFactory};

pub struct Ping {
    base: common::BasePing,
    client_v4: Arc<Client>,
    client_v6: Arc<Client>,
}

pub struct IcmpingHandlerFactory {
    base: common::BasePing,
    client_v4: Arc<Client>,
    client_v6: Arc<Client>,
}

impl HandlerFactory for IcmpingHandlerFactory {
    fn create_handler(&self, addr: SocketAddr) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let (csv, bar, args, success_count, tested_count, timeout_flag) = self.base.clone_shared_state();
        let client_v4 = Arc::clone(&self.client_v4);
        let client_v6 = Arc::clone(&self.client_v6);
        let total_ips = self.base.ip_buffer.total_expected();

        Box::pin(async move {
            let ip = addr.ip();
            let ping_times = args.ping_times;
            
            let mut recv = 0;
            let mut total_delay_ms = 0.0;
            
            // 根据IP类型选择客户端
            let client = match ip {
                IpAddr::V4(_) => client_v4,
                IpAddr::V6(_) => client_v6,
            };
            
            for _ in 0..ping_times {
                // 检查超时信号，如果超时则立即退出
                if timeout_flag.load(Ordering::Relaxed) {
                    break;
                }
                
                if let Ok(Some(delay)) = execute_with_rate_limit(|| async {
                    Ok::<Option<f32>, io::Error>(icmp_ping(addr, &args, &client).await)
                }).await {
                    recv += 1;
                    total_delay_ms += delay;
                }
            }

            // 计算平均延迟
            let avg_delay_ms = common::calculate_precise_delay(total_delay_ms, recv);

            let success_count_override = if recv > 0 {
                let new_success_count = success_count.fetch_add(1, Ordering::Relaxed) + 1;
                let data = PingData::new(addr, ping_times, recv, avg_delay_ms);
                
                if common::should_keep_result(&data, &args) {
                    let mut csv_guard = csv.lock().unwrap();
                    csv_guard.push(data);
                }
                Some(new_success_count)
            } else {
                None
            };
            
            // 更新进度条
            common::update_progress_bar(&bar, &tested_count, &success_count, total_ips, success_count_override);
        })
    }
}

impl Ping {
    pub async fn new(args: &Args, timeout_flag: Arc<AtomicBool>) -> io::Result<Self> {
        // 打印开始延迟测试的信息
        common::print_speed_test_info("ICMP-Ping", args);
        
        // 初始化测试环境
        let base = common::create_base_ping(args, timeout_flag);
        let client_v4 = Arc::new(Client::new(&Config::default())?);
        let client_v6 = Arc::new(Client::new(&Config::builder().kind(ICMP::V6).build())?);

        Ok(Ping {
            base,
            client_v4,
            client_v6,
        })
    }

    fn make_handler_factory(&self) -> Arc<dyn HandlerFactory> {
        Arc::new(IcmpingHandlerFactory {
            base: self.base.clone(),
            client_v4: Arc::clone(&self.client_v4),
            client_v6: Arc::clone(&self.client_v6),
        })
    }

    pub async fn run(self) -> Result<PingDelaySet, io::Error> {
        let handler_factory = self.make_handler_factory();
        common::run_ping_test(&self.base, handler_factory).await
    }
}

// ICMP ping函数
async fn icmp_ping(addr: SocketAddr, args: &Arc<Args>, client: &Arc<Client>) -> Option<f32> {
    let ip = addr.ip();
    let payload = [0; 56];
    let identifier = PingIdentifier(random::<u16>());
    let mut rtt = None;

    let mut pinger = client.pinger(ip, identifier).await;
    pinger.timeout(args.max_delay);

    match pinger.ping(PingSequence(0), &payload).await {
        Ok((packet, dur)) => {
            rtt = Some(dur.as_secs_f32() * 1000.0);
            drop(packet);
        },
        Err(_) => {}
    }
    rtt
}