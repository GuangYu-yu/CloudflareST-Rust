use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crate::hyper::{parse_url_to_uri, RequestContext};
use crate::args::Args;
use crate::common::{self, PingData, BasePing, Ping as CommonPing, PingMode};

#[derive(Clone)]
pub(crate) struct HttpingFactoryData {
    colo_filters: Arc<Vec<String>>,
    original_uri: http::Uri,
    allowed_codes: Option<Arc<Vec<u16>>>,
    host_header: Arc<str>,
    request_context: Arc<RequestContext>,
}

impl common::PingMode for HttpingFactoryData {
    fn run_test(
        &self,
        base: BasePing,
        addr: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = Option<PingData>> + Send>> {
        let args = base.args.clone();
        let colo_filters = self.colo_filters.clone();
        let allowed_codes = self.allowed_codes.clone();
        let original_uri = self.original_uri.clone();
        let host_header = self.host_header.clone();
        let request_context = self.request_context.clone();

        Box::pin(async move {
            let ping_times = args.ping_times;

            let task = Arc::new(PingTask {
                request_context,
                host_header,
                original_uri,
                addr,
                colo_filters,
                allowed_codes,
                should_continue: AtomicBool::new(true),
                local_data_center: OnceLock::new(),
            });

            let (avg_delay, recv) = common::run_ping_loop(ping_times, common::PING_INTERVAL_MS, {
                let task = task.clone();
                move || {
                    let task = task.clone();
                    Box::pin(async move {
                        task.perform_ping().await
                    })
                }
            }).await;

            if !task.should_continue.load(Ordering::Relaxed) {
                return None;
            }

            let data_center = task.local_data_center.get().cloned();
            common::build_ping_data_result(addr, ping_times, recv, avg_delay.unwrap_or(0.0), data_center)
        })
    }
    
    fn clone_box(&self) -> Box<dyn PingMode> {
        Box::new(self.clone())
    }
}

struct PingTask {
    request_context: Arc<RequestContext>,
    host_header: Arc<str>,
    original_uri: http::Uri,
    addr: SocketAddr,
    colo_filters: Arc<Vec<String>>,
    allowed_codes: Option<Arc<Vec<u16>>>,
    should_continue: AtomicBool,
    local_data_center: OnceLock<[u8; 3]>,
}

impl PingTask {
    async fn perform_ping(&self) -> Option<f32> {
        if !self.should_continue.load(Ordering::Relaxed) {
            return None;
        }

        let result = {
            let _permit = crate::pool::acquire_permit().await;
            let start = Instant::now();
            
            let resp = self.request_context.send_request(
                self.host_header.as_ref(),
                &self.original_uri,
                self.addr,
                &http::Method::HEAD,
            ).await?;
            
            let status = resp.status().as_u16();
            if let Some(ref codes) = self.allowed_codes && !codes.contains(&status) {
                return None;
            }
            
            let dc = common::extract_data_center(resp.headers())?;
            let delay = start.elapsed().as_secs_f32() * 1000.0;
            
            Some((delay, dc))
        };

        match result {
            Some((delay, dc)) => {
                if self.local_data_center.get().is_none() {
                    if !self.colo_filters.is_empty() && !common::is_colo_matched(std::str::from_utf8(&dc).unwrap(), &self.colo_filters) {
                        self.should_continue.store(false, Ordering::Relaxed);
                        return None;
                    }
                    let _ = self.local_data_center.set(dc);
                }
                Some(delay)
            }
            None => None,
        }
    }
}

pub(crate) fn new(args: Arc<Args>, sources: Vec<String>, timeout_flag: Arc<AtomicBool>) -> Option<CommonPing> {
    let httping_url = args.httping.as_deref()?;
    let (uri, host_header) = parse_url_to_uri(httping_url)?;

    let colo_filters = if !args.httping_cf_colo.is_empty() {
        common::parse_colo_filters(&args.httping_cf_colo)
    } else {
        Vec::new()
    };

    // 预解析状态码列表
    let allowed_codes = (!args.httping_code.is_empty()).then(|| {
        Arc::new(
            args.httping_code
                .split(',')
                .filter_map(|s| s.trim().parse::<u16>().ok())
                .collect::<Vec<u16>>()
        )
    });

    common::print_speed_test_info("HTTPing", &args);

    let base = common::create_base_ping(args.clone(), sources, timeout_flag);

    let tls_connector = crate::hyper::build_tls_connector().unwrap();
    let request_context = Arc::new(RequestContext {
        interface_config: args.interface_config.clone(),
        tls_connector,
        connect_timeout_ms: crate::common::CONNECT_TIMEOUT_MS,
        ttfb_timeout_ms: crate::common::TTFB_TIMEOUT_MS,
    });

    let factory_data = HttpingFactoryData {
        colo_filters: Arc::new(colo_filters),
        original_uri: uri,
        allowed_codes,
        host_header: host_header.into(),
        request_context,
    };

    Some(CommonPing::new(base, factory_data))
}