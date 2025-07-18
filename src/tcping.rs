use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::net::TcpStream;
use std::io;
use std::sync::atomic::{AtomicUsize, AtomicBool, Ordering};
use std::future::Future;
use std::pin::Pin;

use crate::progress::Bar;
use crate::args::Args;
use crate::pool::execute_with_rate_limit;
use crate::common::{self, PingData, PingDelaySet, HandlerFactory};

// Ping 主体结构体
pub struct Ping {
    base: common::BasePing,
}

pub struct TcpingHandlerFactory {
    base: common::BasePing,
}

impl HandlerFactory for TcpingHandlerFactory {
    fn create_handler(&self, addr: SocketAddr) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let (csv, bar, args, success_count) = self.base.clone_shared_state();
        Box::pin(async move {
            tcping_handler(addr, csv, bar, &args, success_count).await;
        })
    }
}

impl Ping {
    pub async fn new(args: &Args, timeout_flag: Arc<AtomicBool>) -> io::Result<Self> {
        // 打印开始延迟测试的信息
        common::print_speed_test_info("Tcping", args);
        
        // 初始化测试环境
        let base = common::create_base_ping(args, timeout_flag);

        Ok(Ping { base })
    }

    fn make_handler_factory(&self) -> Arc<dyn HandlerFactory> {
        Arc::new(TcpingHandlerFactory {
            base: self.base.clone(),
        })
    }

    pub async fn run(self) -> Result<PingDelaySet, io::Error> {
        let handler_factory = self.make_handler_factory();
        common::run_ping_test(&self.base, handler_factory).await
    }
}

// TCP 测速处理函数
async fn tcping_handler(
    addr: SocketAddr,
    csv: Arc<Mutex<PingDelaySet>>,
    bar: Arc<Bar>,
    args: &Arc<Args>,
    success_count: Arc<AtomicUsize>,
) {
    let ping_times = args.ping_times;
    let mut recv = 0;
    let mut total_delay_ms = 0.0;

    for _ in 0..ping_times {
        if let Ok(Some(delay)) = execute_with_rate_limit(|| async move {
            Ok::<Option<f32>, io::Error>(tcping(addr).await)
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

// TCP连接测试函数
async fn tcping(addr: SocketAddr) -> Option<f32> {
    let connect_result = tokio::time::timeout(
        std::time::Duration::from_millis(2000), // 超时时间
        async {
            let start_time = Instant::now();
            match TcpStream::connect(&addr).await {
                Ok(stream) => {
                    let _ = stream.set_linger(None);
                    drop(stream);
                    Some(start_time.elapsed().as_secs_f32() * 1000.0)
                }
                Err(_) => None,
            }
        },
    ).await;

    connect_result.unwrap_or(None)
}