use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::io;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use surge_ping::{Client, Config, PingIdentifier, PingSequence, ICMP};
use fastrand;
use crate::pool::execute_with_rate_limit;

use crate::args::Args;
use crate::common::{self, PingData, HandlerFactory, BasePing, Ping as CommonPing, PingMode};

#[derive(Clone)]
pub struct IcmpingFactoryData {
    client_v4: Arc<Client>,
    client_v6: Arc<Client>,
}

impl PingMode for IcmpingFactoryData {
    type Handler = IcmpingHandlerFactory;

    fn create_handler_factory(&self, base: &BasePing) -> Arc<Self::Handler> {
        Arc::new(IcmpingHandlerFactory {
            base: Arc::new(base.clone()),
            client_v4: Arc::clone(&self.client_v4),
            client_v6: Arc::clone(&self.client_v6),
        })
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
    let identifier = PingIdentifier(fastrand::u16(0..=u16::MAX));
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