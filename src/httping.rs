use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

use crate::hyper::{parse_url_to_uri, build_hyper_client, send_head_request};
use crate::args::Args;
use crate::common::{self, HandlerFactory, PingData, BasePing, Ping as CommonPing};
use crate::pool::execute_with_rate_limit;
use crate::interface::InterfaceParamResult;

#[derive(Clone)]
pub(crate) struct HttpingFactoryData {
    colo_filters: Arc<Vec<String>>,
    use_https: bool,
    interface_config: Arc<InterfaceParamResult>,
    allowed_codes: Option<Arc<Vec<u16>>>,
}

impl common::PingMode for HttpingFactoryData {
    fn create_handler_factory(&self, base: &BasePing) -> Arc<dyn HandlerFactory> {
        const TRACE_URL_PATH: &str = "cp.cloudflare.com/cdn-cgi/trace";
        let trace_url = format!("{}://{}", if self.use_https { "https" } else { "http" }, TRACE_URL_PATH);
        let (uri, host_header) = parse_url_to_uri(&trace_url).unwrap();

        Arc::new(HttpingHandlerFactory {
            base: base.clone_to_arc(),
            colo_filters: Arc::clone(&self.colo_filters),
            interface_config: Arc::clone(&self.interface_config),
            allowed_codes: self.allowed_codes.clone(),
            uri,
            host_header: host_header.into(),
        })
    }

    fn clone_box(&self) -> Box<dyn common::PingMode> {
        Box::new(self.clone())
    }
}

struct PingTask {
    client: Arc<crate::hyper::MyHyperClient>,
    args: Arc<Args>,
    host_header: Arc<str>,
    uri: http::Uri,
    colo_filters: Arc<Vec<String>>,
    allowed_codes: Option<Arc<Vec<u16>>>,
    local_data_center: Arc<OnceLock<String>>,
    should_continue: Arc<AtomicBool>,
}

impl PingTask {
    async fn perform_ping(&self) -> Option<f32> {
        // 1. 快速检查退出标志
        if !self.should_continue.load(Ordering::SeqCst) {
            return None;
        }

        // 2. 执行带频率限制的请求
        let result = execute_with_rate_limit(|| async {
            let start = Instant::now();
            
            // 发送 HEAD 请求
            let resp = match send_head_request(&self.client, self.host_header.as_ref(), self.uri.clone(), 1200, false).await {
                Ok(resp) => resp,
                Err(_) => return Ok::<Option<(f32, String)>, io::Error>(None),
            };
            
            // 验证状态码
            let status = resp.status().as_u16();
            if let Some(ref codes) = self.allowed_codes && !codes.contains(&status) {
                return Ok::<Option<(f32, String)>, io::Error>(None);
            }
            
            // 提取数据中心信息并计算延迟
            let dc = match common::extract_data_center(&resp) {
                Some(dc) => dc,
                None => return Ok::<Option<(f32, String)>, io::Error>(None),
            };
            let delay = start.elapsed().as_secs_f32() * 1000.0;
            
            Ok::<Option<(f32, String)>, io::Error>(Some((delay, dc)))
        }).await;

        // 3. 处理结果与 Colo 过滤
        match result {
            Ok(Some((delay, dc))) => {
                if self.local_data_center.get().is_none() {
                    // 检查数据中心（Colo）是否符合过滤要求
                    if !self.args.httping_cf_colo.is_empty() && !common::is_colo_matched(&dc, &self.colo_filters) {
                        self.should_continue.store(false, Ordering::SeqCst);
                        return None;
                    }
                    let _ = self.local_data_center.set(dc);
                }
                Some(delay)
            }
            _ => None,
        }
    }
}

pub(crate) struct HttpingHandlerFactory {
    base: Arc<BasePing>,
    colo_filters: Arc<Vec<String>>,
    interface_config: Arc<InterfaceParamResult>,
    allowed_codes: Option<Arc<Vec<u16>>>,
    uri: http::Uri,
    host_header: Arc<str>,
}

impl HandlerFactory for HttpingHandlerFactory {
    fn create_handler(
        &self,
        addr: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = Option<PingData>> + Send>> {
        // 克隆所需的引用
        let base = self.base.clone();
        let args = Arc::clone(&base.args);
        let colo_filters = Arc::clone(&self.colo_filters);
        let interface_config = Arc::clone(&self.interface_config);
        let allowed_codes = self.allowed_codes.clone();
        let uri = self.uri.clone();
        let host_header = Arc::clone(&self.host_header);

        Box::pin(async move {
            let ping_times = args.ping_times;
            let should_continue = Arc::new(AtomicBool::new(true));
            let local_data_center = Arc::new(OnceLock::new());

            // 获取并使用绑定的网络接口信息
            let client = match build_hyper_client(
                addr,
                &interface_config,
                1800,
            ) {
                Some(client) => Arc::new(client),
                None => return None,
            };

            let task = Arc::new(PingTask {
                client,
                args,
                host_header: Arc::clone(&host_header),
                uri,
                colo_filters,
                allowed_codes,
                local_data_center: local_data_center.clone(),
                should_continue: should_continue.clone(),
            });

            let avg_delay = common::run_ping_loop(ping_times, 200, move || {
                let task = task.clone();
                Box::pin(async move { task.perform_ping().await })
            }).await;

            // 如果因 Colo 不匹配而终止，则不返回结果
            if !should_continue.load(Ordering::SeqCst) {
                return None;
            }

            let data_center = local_data_center.get().cloned();
            common::build_ping_data_result(addr, ping_times, avg_delay.unwrap_or(0.0), data_center)
        })
    }
}

pub(crate) fn new(args: Arc<Args>, sources: Vec<String>, timeout_flag: Arc<AtomicBool>) -> io::Result<CommonPing> {
    // 判断是否使用HTTPS协议
    let use_https = args.httping_https;

    // 解析 Colo 过滤条件
    let colo_filters = if !args.httping_cf_colo.is_empty() {
        common::parse_colo_filters(&args.httping_cf_colo)
    } else {
        Vec::new()
    };

    // 预解析状态码列表
    let allowed_codes = if !args.httping_code.is_empty() {
        Some(Arc::new(
            args.httping_code
                .split(',')
                .filter_map(|s| s.trim().parse::<u16>().ok())
                .collect::<Vec<u16>>()
        ))
    } else {
        None
    };

    // 打印开始延迟测试的信息
    let mode_name = if use_https { "HTTPSing" } else { "HTTPing" };
    common::print_speed_test_info(mode_name, &args);

    let base = common::create_base_ping_blocking(Arc::clone(&args), sources, timeout_flag);

    let factory_data = HttpingFactoryData {
        colo_filters: Arc::new(colo_filters),
        use_https,
        interface_config: Arc::clone(&args.interface_config),
        allowed_codes,
    };

    Ok(CommonPing::new(base, factory_data))
}
