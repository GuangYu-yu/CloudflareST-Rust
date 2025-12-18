use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use http_body_util::Full;
use hyper::body::Bytes;
use hyper_util::client::legacy::Client;

use crate::hyper::{parse_url_to_uri, build_hyper_client, send_head_request};
use crate::args::Args;
use crate::common::{self, HandlerFactory, PingData, BasePing, Ping as CommonPing};
use crate::pool::execute_with_rate_limit;

#[derive(Clone)]
pub struct HttpingFactoryData {
    colo_filters: Arc<Vec<String>>,
    use_https: bool,
    interface: Option<String>,
    allowed_codes: Option<Arc<Vec<u16>>>,
}

// 实现 PingMode Trait
impl common::PingMode for HttpingFactoryData {
    fn create_handler_factory(&self, base: &BasePing) -> Arc<dyn HandlerFactory> {
        const TRACE_URL_PATH: &str = "cdnjs.cloudflare.com/cdn-cgi/trace";
        let trace_url = format!("{}://{}", if self.use_https { "https" } else { "http" }, TRACE_URL_PATH);
        let (uri, host_header) = parse_url_to_uri(&trace_url).unwrap();

        Arc::new(HttpingHandlerFactory {
            base: Arc::new(base.clone()),
            colo_filters: Arc::clone(&self.colo_filters),
            interface: self.interface.clone(),
            allowed_codes: self.allowed_codes.clone(),
            uri,
            host_header,
        })
    }
    
    fn clone_box(&self) -> Box<dyn common::PingMode> {
        Box::new(self.clone())
    }
}

struct PingTask {
    client: Arc<Client<hyper_rustls::HttpsConnector<crate::hyper::InterfaceConnector>, Full<Bytes>>>,
    args: Arc<Args>,
    host_header: String,
    uri: http::Uri,
    colo_filters: Arc<Vec<String>>,
    allowed_codes: Option<Arc<Vec<u16>>>,
    local_data_center: Arc<std::sync::Mutex<Option<String>>>,
    should_continue: Arc<AtomicBool>,
}

impl PingTask {
    async fn perform_ping(&self) -> Option<f32> {
        // 1. 快速检查退出标志
        if !self.should_continue.load(Ordering::Relaxed) {
            return None;
        }

        // 2. 执行带频率限制的请求
        let result = execute_with_rate_limit(|| async {
            let start = Instant::now();
            
            // 发送 HEAD 请求
            let resp = match send_head_request(&self.client, &self.host_header, self.uri.clone(), 1200, false).await {
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
                let mut dc_guard = self.local_data_center.lock().unwrap();
                if dc_guard.is_none() {
                    // 检查数据中心（Colo）是否符合过滤要求
                    if !self.args.httping_cf_colo.is_empty() && !common::is_colo_matched(&dc, &self.colo_filters) {
                        self.should_continue.store(false, Ordering::Relaxed);
                        return None;
                    }
                    *dc_guard = Some(dc);
                }
                Some(delay)
            }
            _ => None,
        }
    }
}

pub struct HttpingHandlerFactory {
    base: Arc<BasePing>,
    colo_filters: Arc<Vec<String>>,
    interface: Option<String>,
    allowed_codes: Option<Arc<Vec<u16>>>,
    uri: http::Uri,
    host_header: String,
}

impl HandlerFactory for HttpingHandlerFactory {
    fn create_handler(
        &self,
        addr: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = Option<PingData>> + Send>> {
        // 克隆所需的引用
        let base = self.base.clone();
        let args = base.args.clone();
        let colo_filters = self.colo_filters.clone();
        let interface = self.interface.clone();
        let allowed_codes = self.allowed_codes.clone();
        let uri = self.uri.clone();
        let host_header = self.host_header.clone();

        Box::pin(async move {
            let ping_times = args.ping_times;
            let should_continue = Arc::new(AtomicBool::new(true));
            let local_data_center = Arc::new(std::sync::Mutex::new(None));

            // 获取并使用绑定的网络接口信息
            let interface_ref = interface.as_deref();
            let client = match build_hyper_client(
                addr,
                interface_ref,
                args.interface_ips.as_ref(),
                1800,
            ) {
                Some(client) => Arc::new(client),
                None => return None,
            };

            let task = Arc::new(PingTask {
                client,
                args: args.clone(),
                host_header: host_header.clone(),
                uri: uri.clone(),
                colo_filters: colo_filters.clone(),
                allowed_codes: allowed_codes.clone(),
                local_data_center: local_data_center.clone(),
                should_continue: should_continue.clone(),
            });

            let avg_delay = common::run_ping_loop(ping_times, 200, move || {
                let task = task.clone();
                Box::pin(async move { task.perform_ping().await })
            }).await;

            // 如果因 Colo 不匹配而终止，则不返回结果
            if !should_continue.load(Ordering::Relaxed) {
                return None;
            }

            if let Some(avg_delay_ms) = avg_delay {
                // 构造 PingData 结构体
                let mut data = PingData::new(addr, ping_times, ping_times, avg_delay_ms);
                if let Some(dc) = local_data_center.lock().unwrap().take() {
                    data.data_center = dc;
                }

                // 返回 Ping 结果
                Some(data)
            } else {
                // 没有成功连接或响应，返回 None
                None
            }
        })
    }
}

pub fn new(args: &Args, timeout_flag: Arc<AtomicBool>) -> io::Result<CommonPing> {
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
    common::print_speed_test_info(mode_name, args);

    // 初始化测试环境
    let base = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(common::create_base_ping(args, timeout_flag))
    });

    let factory_data = HttpingFactoryData {
        colo_filters: Arc::new(colo_filters),
        use_https,
        interface: args.interface.clone(),
        allowed_codes,
    };

    Ok(CommonPing::new(base, factory_data))
}