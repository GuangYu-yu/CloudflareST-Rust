use crate::args::Args;
use crate::ip::IpBuffer;
use crate::progress::Bar;
use crate::pool::GLOBAL_LIMITER;
use tokio::task::JoinSet;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use hyper::Response as HyperResponse;

// 定义通用的 PingData 结构体
pub(crate) struct PingData {
    pub(crate) addr: SocketAddr,
    pub(crate) sent: u16,
    pub(crate) received: u16,
    pub(crate) delay: f32,
    pub(crate) download_speed: Option<f32>,
    pub(crate) data_center: String,
}

pub(crate) struct PingDataRef<'a> {
    pub(crate) addr: &'a SocketAddr,
    pub(crate) sent: u16,
    pub(crate) received: u16,
    pub(crate) delay: f32,
    pub(crate) download_speed: Option<f32>,
    pub(crate) data_center: &'a str,
}

impl<'a> From<&'a PingData> for PingDataRef<'a> {
    fn from(data: &'a PingData) -> Self {
        PingDataRef {
            addr: &data.addr,
            sent: data.sent,
            received: data.received,
            delay: data.delay,
            download_speed: data.download_speed,
            data_center: &data.data_center,
        }
    }
}

impl PingData {
    pub(crate) fn new(addr: SocketAddr, sent: u16, received: u16, delay: f32) -> Self {
        Self {
            addr,
            sent,
            received,
            delay,
            download_speed: None,
            data_center: String::new(),
        }
    }

    pub(crate) fn as_ref(&self) -> PingDataRef<'_> {
        PingDataRef::from(self)
    }
}

impl<'a> PingDataRef<'a> {
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
}

// 打印测速信息的通用函数
pub(crate) fn print_speed_test_info(mode: &str, args: &Args) {
    println!(
        "开始延迟测速（模式：{}, 端口：{}, 范围：{} ~ {} ms, 丢包：{:.2})",
        mode,
        args.tcp_port,
        args.min_delay.as_millis(),
        args.max_delay.as_millis(),
        args.max_loss_rate
    );
}

/// 基础Ping结构体，包含所有公共字段
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

    pub(crate) fn clone_to_arc(&self) -> Arc<Self> {
        Arc::new(Self {
            ip_buffer: Arc::clone(&self.ip_buffer),
            bar: Arc::clone(&self.bar),
            args: Arc::clone(&self.args),
            success_count: Arc::clone(&self.success_count),
            timeout_flag: Arc::clone(&self.timeout_flag),
            tested_count: Arc::clone(&self.tested_count),
        })
    }
}

/// 计算平均延迟，精确到两位小数
pub(crate) fn calculate_precise_delay(total_delay_ms: f32, success_count: u16) -> f32 {
    if success_count == 0 {
        return 0.0;
    }

    // 计算平均值
    let avg_ms = total_delay_ms / success_count as f32;
    // 四舍五入到两位小数
    (avg_ms * 100.0).round() / 100.0
}

/// 从响应中提取数据中心信息
pub(crate) fn extract_data_center(resp: &HyperResponse<hyper::body::Incoming>) -> Option<String> {
    resp.headers()
        .get("cf-ray")?
        .to_str()
        .ok()?
        .rsplit('-')
        .next()
        .map(str::to_owned)
}

/// Ping 初始化
pub(crate) async fn create_base_ping(args: Arc<Args>, sources: Vec<String>, timeout_flag: Arc<AtomicBool>) -> BasePing {
    // 处理 IP 源并创建缓冲区
    let (single_ips, cidr_states, total_expected) = crate::ip::process_ip_sources(sources, &args);
    let ip_buffer = IpBuffer::new(cidr_states, single_ips, total_expected, args.tcp_port);

    // 创建 BasePing 所需各项资源并初始化
    BasePing::new(
        Arc::new(ip_buffer),                                    // IP 缓冲区
        Arc::new(Bar::new(total_expected, "可用:", "")), // 创建进度条
        args,                                                     // 参数包装
        Arc::new(AtomicUsize::new(0)),                          // 成功计数器
        timeout_flag,                                           // 提前中止标记
        Arc::new(AtomicUsize::new(0)),                          // 已测试计数器
    )
}

pub(crate) trait HandlerFactory: Send + Sync + 'static {
    fn create_handler(
        &self,
        addr: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = Option<PingData>> + Send>>;
}

/// 通用的ping测试循环函数
pub(crate) async fn run_ping_loop<F, Fut>(
    ping_times: u16,
    wait_ms: u64,
    mut test_fn: F,
) -> Option<f32>
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

            // 成功时等待指定时间再进行下一次ping
            tokio::time::sleep(tokio::time::Duration::from_millis(wait_ms)).await;
        }
    }

    // 计算平均延迟
    let avg_delay_ms = calculate_precise_delay(total_delay_ms, recv);
    (recv > 0).then_some(avg_delay_ms)
}

pub(crate) trait PingMode: Send + Sync + 'static {
    fn create_handler_factory(&self, base: &BasePing) -> Arc<dyn HandlerFactory>;
    fn clone_box(&self) -> Box<dyn PingMode>;
}

impl Clone for Box<dyn PingMode> {
    fn clone(&self) -> Box<dyn PingMode> {
        (**self).clone_box()
    }
}

pub(crate) fn create_base_ping_blocking(args: Arc<Args>, sources: Vec<String>, timeout_flag: Arc<AtomicBool>) -> BasePing {
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(create_base_ping(Arc::clone(&args), sources, timeout_flag))
    })
}

pub(crate) fn build_ping_data_result(addr: SocketAddr, ping_times: u16, avg_delay_ms: f32, data_center: Option<String>) -> Option<PingData> {
    if avg_delay_ms > 0.0 {
        let mut data = PingData::new(addr, ping_times, ping_times, avg_delay_ms);
        if let Some(dc) = data_center {
            data.data_center = dc;
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

    // 通用的 run 方法
    pub(crate) async fn run(self) -> Result<Vec<PingData>, io::Error> {
        let handler_factory = self.factory_data.create_handler_factory(&self.base);
        run_ping_test(self.base, handler_factory).await
    }
}

/// 运行 ping 测试
pub(crate) async fn run_ping_test(
    base: BasePing,
    handler_factory: Arc<dyn HandlerFactory>,
) -> Result<Vec<PingData>, io::Error>
{
    // 并发限制器最大并发数量
    let pool_concurrency = GLOBAL_LIMITER.get().unwrap().max_concurrent;
    
    // 缓存常用值
    let timeout_flag = &base.timeout_flag;
    let success_count = &base.success_count;
    let bar = &base.bar;
    let args = &base.args;
    let total_ips = base.ip_buffer.total_expected();
    let tn = args.target_num.map(|t| t.min(total_ips)); // 目标数量，不超过总IP数
    
    // 创建异步任务管理器和结果收集器
    let mut tasks = JoinSet::new();
    // 使用 -tn 参数时预分配结果向量容量，否则使用默认容量
    let mut results = tn.map_or(Vec::new(), Vec::with_capacity);

    // 初始启动任务直到达到并发限制或没有更多 IP
    (0..pool_concurrency)
        .map_while(|_| base.ip_buffer.pop())
        .for_each(|addr| {
            let _ = tasks.spawn(handler_factory.create_handler(addr));
        });
    
    // 动态循环处理任务，直到超时或任务耗尽
    while let Some(join_result) = tasks.join_next().await {
        // 检查超时信号或是否达到目标成功数量，满足任一条件则提前退出
        let current_success = success_count.load(Ordering::Relaxed);
        if check_timeout_signal(timeout_flag) 
            || tn.is_some_and(|tn| current_success >= tn) {
            tasks.abort_all();
            break;
        }

        // 处理结果
        let mut success_increment = 0;
        if let Ok(result) = join_result
            && let Some(ping_data) = result.filter(|d| should_keep_result(d, args))
        {
            success_increment = 1;
            results.push(ping_data);
        }

        // 更新测试计数和进度条
        let current_tested = base.tested_count.fetch_add(1, Ordering::Relaxed) + 1;
        if success_increment > 0 {
            success_count.fetch_add(success_increment, Ordering::Relaxed);
        }
        update_progress_bar(bar, current_tested, current_success + success_increment, total_ips);

        // 继续添加新任务
        if let Some(addr) = base.ip_buffer.pop() {
            tasks.spawn(handler_factory.create_handler(addr));
        }
    }

    // 完成进度条并排序结果
    bar.done();
    sort_results(&mut results);

    Ok(results)
}

/// 解析数据中心过滤条件字符串为向量
pub(crate) fn parse_colo_filters(colo_filter: &str) -> Vec<String> {
    colo_filter
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

// 检查数据中心是否匹配过滤条件
pub(crate) fn is_colo_matched(data_center: &str, colo_filters: &[String]) -> bool {
    !data_center.is_empty()
        && (colo_filters.is_empty()
            || colo_filters
                .iter()
                .any(|filter| filter.eq_ignore_ascii_case(data_center)))
}

/// 判断测试结果是否符合筛选条件
pub(crate) fn should_keep_result(data: &PingData, args: &Args) -> bool {
    let data_ref = data.as_ref();
    
    // 检查丢包率和延迟上下限
    data_ref.loss_rate() <= args.max_loss_rate
        && data_ref.delay >= args.min_delay.as_millis() as f32
        && data_ref.delay <= args.max_delay.as_millis() as f32
}

/// 排序结果
pub(crate) fn sort_results(results: &mut [PingData]) {
    if results.is_empty() {
        return;
    }

    let (total_count, total_speed, total_loss, total_delay) = {
        let count = results.len() as f32;
        let (speed, loss, delay) = results.iter().fold((0.0, 0.0, 0.0), |acc, d| {
            let d_ref = d.as_ref();
            (
                acc.0 + d_ref.download_speed.unwrap_or(0.0),
                acc.1 + d_ref.loss_rate(),
                acc.2 + d_ref.delay,
            )
        });
        (count, speed, loss, delay)
    };

    let avg_speed = total_speed / total_count;
    let avg_loss = total_loss / total_count;
    let avg_delay = total_delay / total_count;

    let has_speed = results.iter().any(|r| r.download_speed.is_some());

    // 计算分数
    let score = |d: &PingData| {
        let d_ref = d.as_ref();
        let speed = d_ref.download_speed.unwrap_or(0.0);
        let loss = d_ref.loss_rate();
        let delay = d_ref.delay;

        if has_speed {
            (speed - avg_speed) * 0.5 + (delay - avg_delay) * -0.2 + (loss - avg_loss) * -0.3
        } else {
            (delay - avg_delay) * -0.4 + (loss - avg_loss) * -0.6
        }
    };

    results.sort_unstable_by(|a, b| {
        score(b)
            .partial_cmp(&score(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// 检查是否收到超时信号
pub(crate) fn check_timeout_signal(timeout_flag: &AtomicBool) -> bool {
    timeout_flag.load(Ordering::SeqCst)
}

/// 统一的进度条更新函数
pub(crate) fn update_progress_bar(
    bar: &Bar,
    current_tested: usize,
    success_count: usize,
    total_ips: usize,
) {
    bar.update_all(1, format!("{}/{}", current_tested, total_ips), success_count.to_string());
}