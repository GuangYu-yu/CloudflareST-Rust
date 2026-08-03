use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crate::hyper::{parse_url_to_uri, HttpConnection, RequestContext};
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

            let mut conn: Option<HttpConnection> = None;
            let mut recv = 0u16;
            let mut total_delay_ms = 0.0f32;

            for _ in 0..ping_times {
                if !task.should_continue.load(Ordering::Relaxed) {
                    break;
                }
                if let Some(delay) = task.perform_ping(&mut conn).await {
                    recv += 1;
                    total_delay_ms += delay;
                    tokio::time::sleep(tokio::time::Duration::from_millis(common::PING_INTERVAL_MS)).await;
                }
            }

            if !task.should_continue.load(Ordering::Relaxed) {
                return None;
            }

            let avg_delay = common::calculate_precise_delay(total_delay_ms, recv);
            let data_center = task.local_data_center.get().cloned();
            common::build_ping_data_result(addr, ping_times, recv, avg_delay, data_center)
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
    async fn perform_ping(&self, conn_slot: &mut Option<HttpConnection>) -> Option<f32> {
        if !self.should_continue.load(Ordering::Relaxed) {
            return None;
        }

        let _permit = crate::pool::acquire_permit().await;

        // 首次或上次连接已作废时，建立新连接
        if conn_slot.is_none() {
            *conn_slot = self.request_context.open(
                self.host_header.as_ref(),
                &self.original_uri,
                self.addr,
            ).await;
        }
        // 取出连接：请求失败/超时后直接丢弃（HTTP/1 无法部分取消），否则放回复用
        let mut connection = conn_slot.take()?;

        let start = Instant::now();

        let resp = match tokio::time::timeout(
            std::time::Duration::from_millis(self.request_context.ttfb_timeout_ms),
            connection.request(&self.original_uri, &http::Method::HEAD),
        ).await {
            Ok(Ok(resp)) => resp,
            // 超时或 IO 错误：连接已损坏，drop 后保持 None，下次重建
            _ => return None,
        };

        // 业务判断（此时连接仍健康）
        let status = resp.status().as_u16();
        let result = if let Some(ref codes) = self.allowed_codes && !codes.contains(&status) {
            None
        } else {
            common::extract_data_center(resp.headers())
                .map(|dc| (start.elapsed().as_secs_f32() * 1000.0, dc))
        };

        *conn_slot = Some(connection);

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
        ttfb_timeout_ms: crate::common::TTFB_TIMEOUT_MS.max(args.max_delay.as_millis() as u64),
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