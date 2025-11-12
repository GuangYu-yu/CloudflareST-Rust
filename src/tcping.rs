use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use crate::args::Args;
use crate::common::{self, HandlerFactory, PingData, BasePing, Ping as CommonPing, PingMode};
use crate::pool::execute_with_rate_limit;
use crate::interface::{InterfaceIps, bind_socket_to_interface};

#[derive(Clone)]
pub struct TcpingFactoryData {
    interface: Option<String>,
    interface_ips: Option<InterfaceIps>,
}

impl PingMode for TcpingFactoryData {
    type Handler = TcpingHandlerFactory;

    fn create_handler_factory(&self, base: &BasePing) -> Arc<Self::Handler> {
        Arc::new(TcpingHandlerFactory {
            base: Arc::new(base.clone()),
            interface: self.interface.clone(),
            interface_ips: self.interface_ips.clone(),
        })
    }
}

pub struct TcpingHandlerFactory {
    base: Arc<BasePing>,
    interface: Option<String>,
    interface_ips: Option<InterfaceIps>,
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
            
            // 使用通用的ping循环函数
            let avg_delay = common::run_ping_loop(ping_times, 200, || async {
                let interface_ref = interface.as_deref();
                let interface_ips_ref = interface_ips.as_ref();
                
                (execute_with_rate_limit(|| async move {
                    Ok::<Option<f32>, io::Error>(
                        tcping(addr, interface_ref, interface_ips_ref).await,
                    )
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

pub fn new(args: &Args, timeout_flag: Arc<AtomicBool>) -> io::Result<CommonPing<TcpingFactoryData>> {
    // 打印开始延迟测试的信息
    common::print_speed_test_info("Tcping", args);

    // 初始化测试环境
    let base = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(common::create_base_ping(args, timeout_flag))
    });

    let factory_data = TcpingFactoryData {
        interface: args.interface.clone(),
        interface_ips: args.interface_ips.clone(),
    };

    Ok(CommonPing::new(base, factory_data))
}

// TCP连接测试函数
pub async fn tcping(
    addr: SocketAddr,
    interface: Option<&str>,
    interface_ips: Option<&InterfaceIps>,
) -> Option<f32> {
    let start_time = Instant::now();

    // 使用通用的接口绑定函数创建socket
    let socket = bind_socket_to_interface(addr, interface, interface_ips).await?;

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