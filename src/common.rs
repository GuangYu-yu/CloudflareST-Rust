use crate::args::Args;
use crate::ip::IpBuffer;
use crate::progress::Bar;
use crate::pool;
use tokio::task::JoinSet;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// 首字节超时时间（毫秒）
pub(crate) const TTFB_TIMEOUT_MS: u64 = 1000;
/// 连接超时时间（毫秒）
pub(crate) const CONNECT_TIMEOUT_MS: u64 = 1000;
/// 延迟测试中每次 ping 之间的等待间隔（毫秒）
pub(crate) const PING_INTERVAL_MS: u64 = 200;
/// 四舍五入精度（2 位小数）
pub(crate) const ROUNDING_PRECISION: f32 = 100.0;
/// 1 MB 对应的字节数
pub(crate) const BYTES_PER_MB: f32 = 1024.0 * 1024.0;

#[derive(Clone, Copy, Debug)]
pub(crate) struct PingData {
    pub(crate) addr: SocketAddr,
    pub(crate) sent: u16,
    pub(crate) received: u16,
    pub(crate) delay: f32,
    pub(crate) download_speed: Option<f32>,
    pub(crate) data_center: Option<[u8; 3]>,
}

impl PingData {
    pub(crate) fn new(addr: SocketAddr, sent: u16, received: u16, delay: f32) -> Self {
        Self {
            addr,
            sent,
            received,
            delay,
            download_speed: None,
            data_center: None,
        }
    }

    pub(crate) fn loss_rate(&self) -> f32 {
        if self.sent == 0 {
            return 0.0;
        }
        1.0 - (self.received as f32 / self.sent as f32)
    }

    pub(crate) fn display_addr(&self, show_port: bool) -> String {
        if show_port {
            self.addr.to_string()
        } else {
            self.addr.ip().to_string()
        }
    }

    pub(crate) fn colo_str(&self) -> &str {
        colo_opt_to_str(self.data_center.as_ref())
    }
}

pub(crate) fn print_speed_test_info(mode: &str, args: &Args) {
    println!(
        "开始延迟测速（模式：{mode}, 端口：{}, 范围：{} ~ {} ms, 丢包：{:.2})",
        args.tcp_port,
        args.min_delay.as_millis(),
        args.max_delay.as_millis(),
        args.max_loss_rate
    );
}

#[derive(Clone)]
pub(crate) struct BasePing {
    pub(crate) ip_buffer: Arc<IpBuffer>,
    pub(crate) bar: Arc<Bar>,
    pub(crate) args: Arc<Args>,
    pub(crate) success_count: Arc<AtomicUsize>,
    pub(crate) timeout_flag: Arc<AtomicBool>,
    pub(crate) tested_count: Arc<AtomicUsize>,
}

impl BasePing {
    pub(crate) fn new(
        ip_buffer: Arc<IpBuffer>,
        bar: Arc<Bar>,
        args: Arc<Args>,
        success_count: Arc<AtomicUsize>,
        timeout_flag: Arc<AtomicBool>,
        tested_count: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            ip_buffer,
            bar,
            args,
            success_count,
            timeout_flag,
            tested_count,
        }
    }
}

/// 计算平均延迟，精确到两位小数
pub(crate) fn calculate_precise_delay(total_delay_ms: f32, success_count: u16) -> f32 {
    if success_count == 0 {
        return 0.0;
    }

    let avg_ms = total_delay_ms / success_count as f32;
    (avg_ms * ROUNDING_PRECISION).round() / ROUNDING_PRECISION
}

pub(crate) fn extract_data_center(headers: &http::HeaderMap) -> Option<[u8; 3]> {
    let s = headers.get("cf-ray")?.to_str().ok()?.rsplit('-').next()?;
    let b = s.as_bytes();
    (b.len() == 3).then(|| [b[0], b[1], b[2]])
}

pub(crate) fn colo_bytes_to_str(b: &[u8; 3]) -> &str {
    std::str::from_utf8(b).unwrap()
}

/// 将 Option<colo> 转换为字符串（None 返回空串）
pub(crate) fn colo_opt_to_str(opt: Option<&[u8; 3]>) -> &str {
    opt.map_or("", colo_bytes_to_str)
}

/// Ping 初始化
pub(crate) fn create_base_ping(args: Arc<Args>, sources: Vec<String>, timeout_flag: Arc<AtomicBool>) -> BasePing {
    let (single_ips, cidr_states, total_expected) = crate::ip::process_ip_sources(sources, &args);
    let ip_buffer = IpBuffer::new(cidr_states, single_ips, total_expected, args.tcp_port);

    BasePing::new(
        Arc::new(ip_buffer),
        Arc::new(Bar::new(total_expected, "可用:", "")),
        args,
        Arc::new(AtomicUsize::new(0)),
        timeout_flag,
        Arc::new(AtomicUsize::new(0)),
    )
}

pub(crate) async fn run_ping_loop<F, Fut>(
    ping_times: u16,
    wait_ms: u64,
    mut test_fn: F,
) -> (Option<f32>, u16)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<f32>>,
{
    let mut recv = 0;
    let mut total_delay_ms = 0.0;

    for _ in 0..ping_times {
        if let Some(delay) = test_fn().await {
            recv += 1;
            total_delay_ms += delay;

            tokio::time::sleep(tokio::time::Duration::from_millis(wait_ms)).await;
        }
    }

    let avg_delay_ms = calculate_precise_delay(total_delay_ms, recv);
    ((recv > 0).then_some(avg_delay_ms), recv)
}

/// 通用 ping 编排：循环探测 + 构建结果（无 colo）
pub(crate) async fn ping_one<F, Fut>(
    args: Arc<Args>,
    addr: SocketAddr,
    wait_ms: u64,
    test_fn: F,
) -> Option<PingData>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<f32>>,
{
    let ping_times = args.ping_times;
    let (avg_delay, recv) = run_ping_loop(ping_times, wait_ms, test_fn).await;
    build_ping_data_result(addr, ping_times, recv, avg_delay.unwrap_or(0.0), None)
}

pub(crate) trait PingMode: Send + 'static {
    fn run_test(
        &self,
        base: BasePing,
        addr: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = Option<PingData>> + Send>>;
    
    fn clone_box(&self) -> Box<dyn PingMode>;
}

impl Clone for Box<dyn PingMode> {
    fn clone(&self) -> Box<dyn PingMode> {
        (**self).clone_box()
    }
}

pub(crate) fn build_ping_data_result(addr: SocketAddr, sent: u16, received: u16, avg_delay_ms: f32, data_center: Option<[u8; 3]>) -> Option<PingData> {
    if avg_delay_ms > 0.0 {
        let mut data = PingData::new(addr, sent, received, avg_delay_ms);
        if let Some(dc) = data_center {
            data.data_center = Some(dc);
        }
        Some(data)
    } else {
        None
    }
}

pub(crate) struct Ping {
    pub(crate) base: BasePing,
    pub(crate) factory_data: Box<dyn PingMode>,
}

impl Ping {
    pub(crate) fn new<T: PingMode + Clone + 'static>(base: BasePing, factory_data: T) -> Self {
        Self { 
            base, 
            factory_data: Box::new(factory_data) 
        }
    }

    pub(crate) async fn run(self) -> Vec<PingData> {
        run_ping_test(self.base, self.factory_data).await
    }
}

/// 运行 ping 测试
pub(crate) async fn run_ping_test(
    base: BasePing,
    mode: Box<dyn PingMode>,
) -> Vec<PingData>
{
    let pool_concurrency = pool::max_concurrent();

    let timeout_flag = &base.timeout_flag;
    let success_count = &base.success_count;
    let bar = &base.bar;
    let args = &base.args;
    let total_ips = base.ip_buffer.total_expected();
    let tn = args.target_num.map(|t| t.min(total_ips));

    let mut tasks = JoinSet::new();
    let mut results = tn.map_or(Vec::new(), Vec::with_capacity);

    (0..pool_concurrency)
        .map_while(|_| base.ip_buffer.pop())
        .for_each(|addr| {
            let _ = tasks.spawn(mode.run_test(base.clone(), addr));
        });
    
    while let Some(join_result) = tasks.join_next().await {
        let current_success = success_count.load(Ordering::Relaxed);
        if check_timeout_signal(timeout_flag) 
            || tn.is_some_and(|tn| current_success >= tn) {
            tasks.abort_all();
            break;
        }

        let mut success_increment = 0;
        if let Ok(result) = join_result
            && let Some(ping_data) = result.filter(|d| should_keep_result(d, args))
        {
            success_increment = 1;
            results.push(ping_data);
        }

        let current_tested = base.tested_count.fetch_add(1, Ordering::Relaxed) + 1;
        if success_increment > 0 {
            success_count.fetch_add(success_increment, Ordering::Relaxed);
        }
        update_progress_bar(bar, current_tested, current_success + success_increment, total_ips);

        if let Some(addr) = base.ip_buffer.pop() {
            let _ = tasks.spawn(mode.run_test(base.clone(), addr));
        }
    }

    bar.done();
    sort_results(&mut results);

    results
}

pub(crate) fn parse_colo_filters(colo_filter: &str) -> Vec<String> {
    colo_filter
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub(crate) fn is_colo_matched(data_center: &str, colo_filters: &[String]) -> bool {
    !data_center.is_empty()
        && (colo_filters.is_empty()
            || colo_filters.iter().any(|f| f.eq_ignore_ascii_case(data_center)))
}

pub(crate) fn should_keep_result(data: &PingData, args: &Args) -> bool {
    data.loss_rate() <= args.max_loss_rate
        && data.delay >= args.min_delay.as_millis() as f32
        && data.delay <= args.max_delay.as_millis() as f32
}

pub(crate) fn sort_results(results: &mut [PingData]) {
    if results.len() < 2 {
        return;
    }

    let n = results.len() as f32;

    let avg_delay = results.iter().map(|r| r.delay).sum::<f32>() / n;
    let avg_loss  = results.iter().map(|r| r.loss_rate()).sum::<f32>() / n;
    let std_delay = (results.iter().map(|r| (r.delay - avg_delay).powi(2)).sum::<f32>() / n).sqrt();
    let std_loss  = (results.iter().map(|r| (r.loss_rate() - avg_loss).powi(2)).sum::<f32>() / n).sqrt();

    let speeds: Vec<f32> = results.iter().filter_map(|r| r.download_speed).collect();
    let (avg_speed, std_speed) = if !speeds.is_empty() {
        let avg = speeds.iter().sum::<f32>() / speeds.len() as f32;
        let var = speeds.iter().map(|s| (s - avg).powi(2)).sum::<f32>() / speeds.len() as f32;
        (avg, var.sqrt())
    } else {
        (0.0, 0.0)
    };

    // σ < 1e-3 时视为无区分度
    let safe_div = |v: f32, d: f32| if d < 1e-3 { 0.0 } else { v / d };

    const W_SPEED: f32 = 1.0;
    const W_DELAY: f32 = 1.0;
    const W_LOSS:  f32 = 1.0;

    let score = |d: &PingData| {
        // z-score：各维度归一化后加权求和，None 表示"该维度信息缺失"而非"值为零"
        let z_speed = match d.download_speed {
            Some(s) => safe_div(s - avg_speed, std_speed),
            None    => 0.0,
        };
        let z_delay = safe_div(d.delay - avg_delay, std_delay);
        let z_loss  = safe_div(d.loss_rate() - avg_loss, std_loss);
        W_SPEED * z_speed - W_DELAY * z_delay - W_LOSS * z_loss
    };

    results.sort_unstable_by(|a, b| score(b).total_cmp(&score(a)));
}

pub(crate) fn check_timeout_signal(timeout_flag: &AtomicBool) -> bool {
    timeout_flag.load(Ordering::SeqCst)
}

pub(crate) fn update_progress_bar(
    bar: &Bar,
    current_tested: usize,
    success_count: usize,
    total_ips: usize,
) {
    bar.update(current_tested, format!("{}/{}", current_tested, total_ips), success_count.to_string());
}