use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use crate::args::Args;
use crate::common::{self, HandlerFactory, PingData, PingDelaySet};
use crate::pool::execute_with_rate_limit;

// Ping 主体结构体
pub struct Ping {
    base: common::BasePing,
}

pub struct TcpingHandlerFactory {
    base: common::BasePing,
    interface: Option<String>,
    interface_ips: Option<crate::interface::InterfaceIps>,
}

impl HandlerFactory for TcpingHandlerFactory {
    fn create_handler(
        &self,
        addr: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = Option<PingData>> + Send>> {
        let args = Arc::clone(&self.base.args);
        let interface = self.interface.clone();
        let interface_ips = self.interface_ips.clone();

        Box::pin(async move {
            let ping_times = args.ping_times;
            let mut recv = 0;
            let mut total_delay_ms = 0.0;

            for _ in 0..ping_times {
                let interface_ref = interface.as_deref();
                let interface_ips_ref = interface_ips.as_ref();
                if let Ok(Some(delay)) = execute_with_rate_limit(|| async move {
                    Ok::<Option<f32>, io::Error>(
                        tcping(addr, interface_ref, interface_ips_ref).await,
                    )
                })
                .await
                {
                    recv += 1;
                    total_delay_ms += delay;

                    // 成功时等待300ms再进行下一次ping
                    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
                }
            }

            // 计算平均延迟
            let avg_delay_ms = common::calculate_precise_delay(total_delay_ms, recv);

            if recv > 0 {
                let data = PingData::new(addr, ping_times, recv, avg_delay_ms);
                // 返回测试数据
                Some(data)
            } else {
                None
            }
        })
    }
}

impl Ping {
    pub async fn new(args: &Args, timeout_flag: Arc<AtomicBool>) -> io::Result<Self> {
        // 打印开始延迟测试的信息
        common::print_speed_test_info("Tcping", args);

        // 初始化测试环境
        let base = common::create_base_ping(args, timeout_flag).await;

        Ok(Ping { base })
    }

    fn make_handler_factory(&self) -> Arc<dyn HandlerFactory> {
        Arc::new(TcpingHandlerFactory {
            base: self.base.clone(),
            interface: self.base.args.interface.clone(),
            interface_ips: self.base.args.interface_ips.clone(),
        })
    }

    pub async fn run(self) -> Result<PingDelaySet, io::Error> {
        let handler_factory = self.make_handler_factory();
        common::run_ping_test(&self.base, handler_factory).await
    }
}

// TCP连接测试函数
pub async fn tcping(
    addr: SocketAddr,
    interface: Option<&str>,
    interface_ips: Option<&crate::interface::InterfaceIps>,
) -> Option<f32> {
    let start_time = Instant::now();

    // 使用通用的接口绑定函数创建socket
    let socket = crate::interface::bind_socket_to_interface(addr, interface, interface_ips).await?;

    // 连接
    match tokio::time::timeout(std::time::Duration::from_millis(1000), socket.connect(addr)).await {
        Ok(Ok(stream)) => {
            let _ = stream.set_linger(None);
            drop(stream);
            Some(start_time.elapsed().as_secs_f32() * 1000.0)
        }
        _ => None,
    }
}