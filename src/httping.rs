use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use http::Method;

use crate::hyper::{send_request, parse_url_to_uri};
use crate::args::Args;
use crate::common::{self, HandlerFactory, PingData, BasePing, Ping as CommonPing};
use crate::pool::execute_with_rate_limit;

#[derive(Clone)]
pub(crate) struct HttpingFactoryData {
    colo_filters: Arc<Vec<String>>,
    scheme: String,
    path: String,
    allowed_codes: Option<Arc<Vec<u16>>>,
    host_header: String,
    global_client: Arc<crate::hyper::MyHyperClient>,
}

impl common::PingMode for HttpingFactoryData {
    fn create_handler_factory(&self, base: &BasePing) -> Arc<dyn HandlerFactory> {
        Arc::new(HttpingHandlerFactory {
            base: Arc::new(base.clone()),
            colo_filters: Arc::clone(&self.colo_filters),
            allowed_codes: self.allowed_codes.clone(),
            scheme: self.scheme.clone(),
            host_header: self.host_header.clone().into(),
            path: self.path.clone(),
            global_client: Arc::clone(&self.global_client),
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
    should_continue: AtomicBool,
    local_data_center: std::sync::OnceLock<String>,
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
            let resp = match send_request(&self.client, self.host_header.as_ref(), self.uri.clone(), Method::HEAD, 1200).await {
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
                        self.should_continue.store(false, Ordering::Relaxed);
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
    allowed_codes: Option<Arc<Vec<u16>>>,
    scheme: String,
    path: String,
    host_header: Arc<str>,
    global_client: Arc<crate::hyper::MyHyperClient>,
}

impl HandlerFactory for HttpingHandlerFactory {
    fn create_handler(
        &self,
        addr: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = Option<PingData>> + Send>> {
        let base = self.base.clone();
        let args = Arc::clone(&base.args);
        let colo_filters = Arc::clone(&self.colo_filters);
        let allowed_codes = self.allowed_codes.clone();
        
        // 构造 URI
        let uri: http::Uri = format!("{}://{}{}", self.scheme, addr, self.path).parse().unwrap();

        let host_header = Arc::clone(&self.host_header);
        let global_client = Arc::clone(&self.global_client);

        Box::pin(async move {
            let ping_times = args.ping_times;

            // 克隆全局 Client
            let client = Arc::clone(&global_client);

            // 创建任务结构体（包含共享状态）
            let task = Arc::new(PingTask {
                client,
                args,
                host_header: Arc::clone(&host_header),
                uri,
                colo_filters,
                allowed_codes,
                should_continue: AtomicBool::new(true),
                local_data_center: std::sync::OnceLock::new(),
            });

            let avg_delay = common::run_ping_loop(ping_times, 200, {
                let task = Arc::clone(&task);
                move || {
                    let task = Arc::clone(&task);
                    Box::pin(async move {
                        task.perform_ping().await
                    })
                }
            }).await;

            // 如果因 Colo 不匹配而终止，则不返回结果
            if !task.should_continue.load(Ordering::Relaxed) {
                return None;
            }

            let data_center = task.local_data_center.get().cloned();
            common::build_ping_data_result(addr, ping_times, avg_delay.unwrap_or(0.0), data_center)
        })
    }
}

pub(crate) fn new(args: Arc<Args>, sources: Vec<String>, timeout_flag: Arc<AtomicBool>) -> io::Result<CommonPing> {
    // 解析提供的HTTPing URL
    let (uri, host_header) = parse_url_to_uri(&args.httping).unwrap();
    
    let scheme = uri.scheme_str().unwrap();
    let path = uri.path();

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
    let mode_name = "HTTPing";
    common::print_speed_test_info(mode_name, &args);

    let base = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(common::create_base_ping(Arc::clone(&args), sources, timeout_flag))
    });

    let client = crate::hyper::build_hyper_client(
        &args.interface_config,
        1800,
        host_header.to_string(),
    ).unwrap();

    let factory_data = HttpingFactoryData {
        colo_filters: Arc::new(colo_filters),
        scheme: scheme.to_string(),
        path: path.to_string(),
        allowed_codes,
        host_header,
        global_client: Arc::new(client),
    };

    Ok(CommonPing::new(base, factory_data))
}