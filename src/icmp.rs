use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::io;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, AtomicBool, Ordering};
use surge_ping::{Client, Config, PingIdentifier, PingSequence, ICMP};
use rand::random;
use crate::pool::execute_with_rate_limit;

use crate::progress::Bar;
use crate::args::Args;
use crate::common::{self, PingData, PingDelaySet, HandlerFactory};
use crate::ip::IpBuffer;

pub struct Ping {
    ip_buffer: Arc<Mutex<IpBuffer>>,
    csv: Arc<Mutex<PingDelaySet>>,
    bar: Arc<Bar>,
    args: Arc<Args>,
    success_count: Arc<AtomicUsize>,
    client_v4: Arc<Client>,
    client_v6: Arc<Client>,
    timeout_flag: Arc<AtomicBool>,
}

pub struct IcmpingHandlerFactory {
    csv: Arc<Mutex<PingDelaySet>>,
    bar: Arc<Bar>,
    args: Arc<Args>,
    success_count: Arc<AtomicUsize>,
    client_v4: Arc<Client>,
    client_v6: Arc<Client>,
}

impl HandlerFactory for IcmpingHandlerFactory {
    fn create_handler(&self, addr: SocketAddr) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let csv = Arc::clone(&self.csv);
        let bar = Arc::clone(&self.bar);
        let args = Arc::clone(&self.args);
        let success_count = Arc::clone(&self.success_count);
        let client_v4 = Arc::clone(&self.client_v4);
        let client_v6 = Arc::clone(&self.client_v6);

        Box::pin(async move {
            icmp_handler(addr, csv, bar, &args, success_count, client_v4, client_v6).await;
        })
    }
}

impl Ping {
    pub async fn new(args: &Args, timeout_flag: Arc<AtomicBool>) -> io::Result<Self> {
        // 打印开始延迟测试的信息
        common::print_speed_test_info("ICMP-Ping", args);
        // 初始化测试环境
        let (ip_buffer, csv, bar) = common::init_ping_test(args);
        let client_v4 = Arc::new(Client::new(&Config::default())?);
        let client_v6 = Arc::new(Client::new(&Config::builder().kind(ICMP::V6).build())?);

        Ok(Ping {
            ip_buffer,
            csv,
            bar,
            args: Arc::new(args.clone()),
            success_count: Arc::new(AtomicUsize::new(0)),
            client_v4,
            client_v6,
            timeout_flag,
        })
    }

    fn make_handler_factory(&self) -> Arc<dyn HandlerFactory> {
        Arc::new(IcmpingHandlerFactory {
            csv: Arc::clone(&self.csv),
            bar: Arc::clone(&self.bar),
            args: Arc::clone(&self.args),
            success_count: Arc::clone(&self.success_count),
            client_v4: Arc::clone(&self.client_v4),
            client_v6: Arc::clone(&self.client_v6),
        })
    }

    pub async fn run(self) -> Result<PingDelaySet, io::Error> {
        let handler_factory = self.make_handler_factory();

        common::run_ping_test(
            self.ip_buffer,
            self.csv,
            self.bar,
            self.args,
            self.success_count,
            self.timeout_flag,
            handler_factory,
        ).await
    }
}

// ICMP 测速处理函数
async fn icmp_handler(
    addr: SocketAddr,
    csv: Arc<Mutex<PingDelaySet>>, 
    bar: Arc<Bar>, 
    args: &Arc<Args>,
    success_count: Arc<AtomicUsize>,
    client_v4: Arc<Client>,
    client_v6: Arc<Client>,
) {
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
        if let Ok(Some(delay)) = execute_with_rate_limit(|| async {
            Ok::<Option<f32>, io::Error>(icmp_ping(addr, args, &client).await)
        }).await {
            recv += 1;
            total_delay_ms += delay;
        }
    }

    // 计算平均延迟
    let avg_delay_ms = common::calculate_precise_delay(total_delay_ms, recv);

    if recv > 0 {
        success_count.fetch_add(1, Ordering::Relaxed);
        let data = PingData::new(addr, ping_times, recv, avg_delay_ms);
        
        if common::should_keep_result(&data, args) {
            let mut csv_guard = csv.lock().unwrap();
            csv_guard.push(data);
        }
    }
    
    // 无论成功与否，每个IP测试完成后都增加1个计数
    let current_count = success_count.load(Ordering::Relaxed);
    bar.grow(1, current_count.to_string());
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