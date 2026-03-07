use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use surge_ping::{Client, Config, PingIdentifier, PingSequence, ICMP};

use crate::args::Args;
use crate::common::{self, PingData, BasePing, Ping as CommonPing, PingMode};
use crate::pool::execute_with_rate_limit;

// 标识符计数器
static PING_IDENTIFIER_COUNTER: AtomicU16 = AtomicU16::new(0);

#[derive(Clone)]
pub(crate) struct IcmpingFactoryData {
    client_v4: Arc<Client>,
    client_v6: Arc<Client>,
}

impl PingMode for IcmpingFactoryData {
    fn run_test(
        &self,
        base: BasePing,
        addr: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = Option<PingData>> + Send>> {
        let args = base.args.clone();
        let ip = addr.ip();
        let client = match ip {
            IpAddr::V4(_) => self.client_v4.clone(),
            IpAddr::V6(_) => self.client_v6.clone(),
        };

        Box::pin(async move {
            let ping_times = args.ping_times;
            
            let avg_delay = common::run_ping_loop(ping_times, 0, || {
                let client = client.clone();
                let args = args.clone();
                async move {
                    execute_with_rate_limit(|| async {
                        icmp_ping(addr, &args, &client).await
                    }).await
                }
            }).await;

            common::build_ping_data_result(addr, ping_times, avg_delay.unwrap_or(0.0), None)
        })
    }
    
    fn clone_box(&self) -> Box<dyn PingMode> {
        Box::new(self.clone())
    }
}

pub(crate) fn new(args: Arc<Args>, sources: Vec<String>, timeout_flag: Arc<AtomicBool>) -> Option<CommonPing> {
    common::print_speed_test_info("ICMP-Ping", &args);

    let base = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(common::create_base_ping(args.clone(), sources, timeout_flag))
    });

    let client_v4 = Arc::new(Client::new(&Config::default()).ok()?);
    let client_v6 = Arc::new(Client::new(&Config::builder().kind(ICMP::V6).build()).ok()?);

    let factory_data = IcmpingFactoryData {
        client_v4,
        client_v6,
    };

    Some(CommonPing::new(base, factory_data))
}

// ICMP ping函数
async fn icmp_ping(addr: SocketAddr, args: &Arc<Args>, client: &Arc<Client>) -> Option<f32> {
    let ip = addr.ip();
    let payload = [0; 56];
    // 生成唯一标识符
    let identifier = PingIdentifier(PING_IDENTIFIER_COUNTER.fetch_add(1, Ordering::Relaxed));
    let mut rtt = None;

    let mut pinger = client.pinger(ip, identifier).await;
    pinger.timeout(args.max_delay);

    if let Ok((_, dur)) = pinger.ping(PingSequence(0), &payload).await {
        rtt = Some(dur.as_secs_f32() * 1000.0);
    }
    rtt
}