use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use crate::args::Args;
use crate::common::{self, PingData, BasePing, Ping as CommonPing, PingMode};
use crate::pool::execute_with_rate_limit;
use crate::interface::{InterfaceParamResult, bind_socket_to_interface};

#[derive(Clone)]
pub(crate) struct TcpingFactoryData {
    interface_config: Arc<InterfaceParamResult>,
}

impl PingMode for TcpingFactoryData {
    fn run_test(
        &self,
        base: BasePing,
        addr: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = Option<PingData>> + Send>> {
        let args = base.args.clone();
        let interface_config = self.interface_config.clone();

        Box::pin(async move {
            let ping_times = args.ping_times;
            
            let avg_delay = common::run_ping_loop(ping_times, 200, || {
                let interface_config = interface_config.clone();
                async move {
                    execute_with_rate_limit(|| async move {
                        tcping(addr, &interface_config).await
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

pub(crate) fn new(args: Arc<Args>, sources: Vec<String>, timeout_flag: Arc<AtomicBool>) -> CommonPing {
    common::print_speed_test_info("Tcping", &args);

    let base = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(common::create_base_ping(args.clone(), sources, timeout_flag))
    });

    let factory_data = TcpingFactoryData {
        interface_config: args.interface_config.clone(),
    };

    CommonPing::new(base, factory_data)
}

// TCP连接测试函数
pub(crate) async fn tcping(
    addr: SocketAddr,
    interface_config: &Arc<InterfaceParamResult>,
) -> Option<f32> {
    let start_time = Instant::now();

    // 使用通用的接口绑定函数创建socket
    let socket = bind_socket_to_interface(addr, interface_config).await?;

    // 连接
    match tokio::time::timeout(std::time::Duration::from_millis(1000), socket.connect(addr)).await {
        Ok(Ok(stream)) => {
            drop(stream);
            Some(start_time.elapsed().as_secs_f32() * 1000.0)
        }
        _ => None,
    }
}