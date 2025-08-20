use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpStream;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::future::Future;
use std::pin::Pin;

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
        let (csv, bar, args, success_count, tested_count) = self.base.clone_shared_state();
        let total_ips = self.base.ip_buffer.total_expected();
        
        Box::pin(async move {
            let ping_times = args.ping_times;
            let mut recv = 0;
            let mut total_delay_ms = 0.0;

            for _ in 0..ping_times {
                if let Ok(Some(delay)) = execute_with_rate_limit(|| async move {
                    Ok::<Option<f32>, io::Error>(tcping(addr).await)
                }).await {
                    recv += 1;
                    total_delay_ms += delay;
                    
                    // 成功时等待100ms再进行下一次ping
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
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

// TCP连接测试函数
async fn tcping(addr: SocketAddr) -> Option<f32> {
    let connect_result = tokio::time::timeout(
        std::time::Duration::from_millis(1000), // 超时时间
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