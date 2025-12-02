use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::io;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use surge_ping::{Client, Config, PingIdentifier, PingSequence, ICMP};
use crate::pool::execute_with_rate_limit;

use crate::args::Args;
use crate::common::{self, PingData, HandlerFactory, BasePing, Ping as CommonPing, PingMode};

// 全局原子计数器，用于生成唯一的ping标识符
static PING_IDENTIFIER_COUNTER: AtomicU16 = AtomicU16::new(0);

#[derive(Clone)]
pub struct IcmpingFactoryData {
    client_v4: Arc<Client>,
    client_v6: Arc<Client>,
}

impl PingMode for IcmpingFactoryData {
    fn create_handler_factory(&self, base: &BasePing) -> Arc<dyn HandlerFactory> {
        Arc::new(IcmpingHandlerFactory {
            base: Arc::new(base.clone()),
            client_v4: Arc::clone(&self.client_v4),
            client_v6: Arc::clone(&self.client_v6),
        })
    }
    
    fn clone_box(&self) -> Box<dyn PingMode> {
        Box::new(self.clone())
    }
}

pub struct IcmpingHandlerFactory {
    base: Arc<BasePing>,
    client_v4: Arc<Client>,
    client_v6: Arc<Client>,
}

impl HandlerFactory for IcmpingHandlerFactory {
    fn create_handler(&self, addr: SocketAddr) -> Pin<Box<dyn Future<Output = Option<PingData>> + Send>> {
        let args = Arc::clone(&self.base.args);
        let client_v4 = Arc::clone(&self.client_v4);
        let client_v6 = Arc::clone(&self.client_v6);
        let ip = addr.ip();

        Box::pin(async move {
            let ping_times = args.ping_times;
            
            // 根据IP类型选择客户端
            let client = match ip {
                IpAddr::V4(_) => client_v4,
                IpAddr::V6(_) => client_v6,
            };
            
            // 使用通用的ping循环函数
            let avg_delay = common::run_ping_loop(ping_times, 0, || async {
                (execute_with_rate_limit(|| async {
                    Ok::<Option<f32>, io::Error>(icmp_ping(addr, &args, &client).await)
                })
                .await).unwrap_or_default()
            }).await;

            if let Some(avg_delay_ms) = avg_delay {
                let data = PingData::new(addr, ping_times, ping_times, avg_delay_ms);
                Some(data)
            } else {
                None
            }
        })
    }
}

pub fn new(args: &Args, timeout_flag: Arc<AtomicBool>) -> io::Result<CommonPing> {
    // 打印开始延迟测试的信息
    common::print_speed_test_info("ICMP-Ping", args);
    
    // 初始化测试环境
    let base = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(common::create_base_ping(args, timeout_flag))
    });

    let client_v4 = Arc::new(Client::new(&Config::default())?);
    let client_v6 = Arc::new(Client::new(&Config::builder().kind(ICMP::V6).build())?);

    let factory_data = IcmpingFactoryData {
        client_v4,
        client_v6,
    };

    Ok(CommonPing::new(base, factory_data))
}

// ICMP ping函数
async fn icmp_ping(addr: SocketAddr, args: &Arc<Args>, client: &Arc<Client>) -> Option<f32> {
    let ip = addr.ip();
    let payload = [0; 56];
    // 生成唯一标识符
    let identifier = PingIdentifier(PING_IDENTIFIER_COUNTER.fetch_add(1, Ordering::SeqCst));
    let mut rtt = None;

    let mut pinger = client.pinger(ip, identifier).await;
    pinger.timeout(args.max_delay);

    match pinger.ping(PingSequence(0), &payload).await {
        Ok((_, dur)) => {
            rtt = Some(dur.as_secs_f32() * 1000.0);
        },
        Err(_) => {}
    }
    rtt
}
