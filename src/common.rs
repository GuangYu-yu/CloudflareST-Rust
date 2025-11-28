use crate::args::Args;
use crate::ip::{IpBuffer, load_ip_to_buffer};
use crate::progress::Bar;
use crate::hyper::{client_builder, parse_url_to_uri};
use crate::pool::GLOBAL_LIMITER;
use tokio::task::JoinSet;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use crate::warning_println;
use crate::hyper::send_get_request_simple;
use hyper::Response as HyperResponse;

// 定义通用的 PingData 结构体
pub struct PingData {
    pub addr: SocketAddr,
    pub sent: u16,
    pub received: u16,
    pub delay: f32,
    pub download_speed: Option<f32>,
    pub data_center: String,
}

pub struct PingDataRef<'a> {
    pub addr: &'a SocketAddr,
    pub sent: u16,
    pub received: u16,
    pub delay: f32,
    pub download_speed: Option<f32>,
    pub data_center: &'a str,
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
    pub fn new(addr: SocketAddr, sent: u16, received: u16, delay: f32) -> Self {
        Self {
            addr,
            sent,
            received,
            delay,
            download_speed: None,
            data_center: String::new(),
        }
    }

    pub fn as_ref(&self) -> PingDataRef<'_> {
        PingDataRef::from(self)
    }
}

impl<'a> PingDataRef<'a> {
    pub fn loss_rate(&self) -> f32 {
        if self.sent == 0 {
            return 0.0;
        }
        1.0 - (self.received as f32 / self.sent as f32)
    }

    pub fn display_addr(&self, show_port: bool) -> String {
        if show_port {
            self.addr.to_string()
        } else {
            self.addr.ip().to_string()
        }
    }
}

// 打印测速信息的通用函数
pub fn print_speed_test_info(mode: &str, args: &Args) {
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
#[derive(Clone)]
pub struct BasePing {
    pub ip_buffer: Arc<IpBuffer>,
    pub bar: Arc<Bar>,
    pub args: Arc<Args>,
    pub success_count: Arc<AtomicUsize>,
    pub timeout_flag: Arc<AtomicBool>,
    pub tested_count: Arc<AtomicUsize>,
}

impl BasePing {
    pub fn new(
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
pub fn calculate_precise_delay(total_delay_ms: f32, success_count: u16) -> f32 {
    if success_count == 0 {
        return 0.0;
    }

    // 计算平均值
    let avg_ms = total_delay_ms / success_count as f32;
    // 四舍五入到两位小数
    (avg_ms * 100.0).round() / 100.0
}

/// 从响应中提取数据中心信息
pub fn extract_data_center(resp: &HyperResponse<hyper::body::Incoming>) -> Option<String> {
    resp.headers()
        .get("cf-ray")?
        .to_str()
        .ok()?
        .rsplit('-')
        .next()
        .map(str::to_owned)
}

/// Ping 初始化
pub async fn create_base_ping(args: &Args, timeout_flag: Arc<AtomicBool>) -> BasePing {
    // 加载 IP 缓冲区
    let ip_buffer = load_ip_to_buffer(args).await;

    // 获取预计总 IP 数量用于进度条
    let total_expected = ip_buffer.total_expected();

    // 创建 BasePing 所需各项资源并初始化
    BasePing::new(
        Arc::new(ip_buffer),                                    // IP 缓冲区
        Arc::new(Bar::new(total_expected, "可用:", "")), // 创建进度条
        Arc::new(args.clone()),                                 // 参数包装
        Arc::new(AtomicUsize::new(0)),                          // 成功计数器
        timeout_flag,                                           // 提前中止标记
        Arc::new(AtomicUsize::new(0)),                          // 已测试计数器
    )
}

pub trait HandlerFactory: Send + Sync + 'static {
    fn create_handler(
        &self,
        addr: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = Option<PingData>> + Send>>;
}

/// 通用的ping测试循环函数
pub async fn run_ping_loop<F, Fut>(
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
    if recv > 0 {
        Some(avg_delay_ms)
    } else {
        None
    }
}

pub trait PingMode: Send + Sync + 'static {
    fn create_handler_factory(&self, base: &BasePing) -> Arc<dyn HandlerFactory>;
    fn clone_box(&self) -> Box<dyn PingMode>;
}

impl Clone for Box<dyn PingMode> {
    fn clone(&self) -> Box<dyn PingMode> {
        (**self).clone_box()
    }
}

pub struct Ping {
    pub base: BasePing,
    pub factory_data: Box<dyn PingMode>,
}

impl Ping {
    pub fn new<T: PingMode + Clone + 'static>(base: BasePing, factory_data: T) -> Self {
        Self { 
            base, 
            factory_data: Box::new(factory_data) 
        }
    }

    // 通用的 run 方法
    pub async fn run(self) -> Result<Vec<PingData>, io::Error> {
        let handler_factory = self.factory_data.create_handler_factory(&self.base);
        run_ping_test(self.base, handler_factory).await
    }
}

/// 运行 ping 测试
pub async fn run_ping_test(
    base: BasePing,
    handler_factory: Arc<dyn HandlerFactory>,
) -> Result<Vec<PingData>, io::Error>
{
    // 并发限制器最大并发数量
    let pool_concurrency = GLOBAL_LIMITER.get().unwrap().max_concurrent;
    
    // 缓存常用值
    let target_num = base.args.target_num;
    let timeout_flag = &base.timeout_flag;
    let success_count = &base.success_count;
    let bar = &base.bar;
    let args = &base.args;
    let total_ips = base.ip_buffer.total_expected();
    
    // 创建异步任务管理器和结果收集器
    let mut tasks = JoinSet::new();
    // 使用 -tn 参数时预分配结果向量容量，否则使用默认容量
    let mut results = target_num.map_or(Vec::new(), |tn| Vec::with_capacity(tn));

    // 初始启动任务直到达到并发限制或没有更多 IP
    for _ in 0..pool_concurrency {
        if let Some(addr) = base.ip_buffer.pop() {
            tasks.spawn(handler_factory.create_handler(addr));
        } else {
            break;
        }
    }
    
    // 动态循环处理任务，直到超时或任务耗尽
    while let Some(join_result) = tasks.join_next().await {
        // 检查超时信号或是否达到目标成功数量，满足任一条件则提前退出
        let current_success = success_count.load(Ordering::Relaxed);
        if check_timeout_signal(timeout_flag) 
            || target_num.is_some_and(|tn| current_success >= tn) {
            tasks.abort_all();
            break;
        }

        // 处理结果
        if let Ok(result) = join_result
            && let Some(ping_data) = result.filter(|d| should_keep_result(d, args))
        {
            success_count.fetch_add(1, Ordering::Relaxed);
            results.push(ping_data);
        }

        // 更新测试计数和进度条
        let current_tested = base.tested_count.fetch_add(1, Ordering::Relaxed) + 1;
        let current_success = success_count.load(Ordering::Relaxed);
        update_progress_bar(bar, current_tested, current_success, total_ips);

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

/// 从URL获取列表
pub async fn get_list(url: &str, max_retries: u8) -> Vec<String> {
    if url.is_empty() {
        return Vec::new();
    }

    // 解析URL获取URI和主机名
    let (uri, host) = match parse_url_to_uri(url) {
        Some((u, h)) => (u, h),
        None => return Vec::new(),
    };

    // 最多尝试指定次数
    for i in 1..=max_retries {
        // 创建客户端
        let mut client = match client_builder() {
            Ok(c) => c,
            Err(_) => continue,
        };

        // 发送 GET 请求
        if let Ok(body_bytes) = send_get_request_simple(&mut client, &host, uri.clone(), 5000).await {
            let content = String::from_utf8_lossy(&body_bytes);
            return content
                .lines()
                .map(|line| line.trim())
                .filter(|line| !line.is_empty() && !line.starts_with("//") && !line.starts_with('#'))
                .map(|line| line.to_string())
                .collect();
        }

        // 重试提示
        if i < max_retries {
            warning_println(format_args!("列表请求失败，正在第{}次重试..", i));
            tokio::time::sleep(Duration::from_secs(1)).await;
        } else {
            warning_println(format_args!("获取列表已达到最大重试次数"));
        }
    }

    Vec::new()
}

/// 从 URL 列表或单一 URL 获取测试 URL 列表
pub async fn get_url_list(url: &str, urlist: &str) -> Vec<String> {
    if !urlist.is_empty() {
        let list = get_list(urlist, 5).await;
        if !list.is_empty() {
            return list;
        }
    }

    // 使用单一URL作为默认值
    if !url.is_empty() {
        vec![url.to_string()]
    } else {
        Vec::new()
    }
}

/// 解析数据中心过滤条件字符串为向量
pub fn parse_colo_filters(colo_filter: &str) -> Vec<String> {
    colo_filter
        .split(',')
        .map(|s| s.trim().to_uppercase())
        .filter(|s| !s.is_empty())
        .collect()
}

// 检查数据中心是否匹配过滤条件
pub fn is_colo_matched(data_center: &str, colo_filters: &[String]) -> bool {
    !data_center.is_empty()
        && (colo_filters.is_empty()
            || colo_filters
                .iter()
                .any(|filter| filter.eq_ignore_ascii_case(data_center)))
}

/// 判断测试结果是否符合筛选条件
#[inline]
pub fn should_keep_result(data: &PingData, args: &Args) -> bool {
    let data_ref = data.as_ref();
    
    // 检查丢包率和延迟上下限
    data_ref.loss_rate() <= args.max_loss_rate
        && data_ref.delay >= args.min_delay.as_millis() as f32
        && data_ref.delay <= args.max_delay.as_millis() as f32
}

/// 排序结果
pub fn sort_results(results: &mut [PingData]) {
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
#[inline]
pub fn check_timeout_signal(timeout_flag: &AtomicBool) -> bool {
    timeout_flag.load(Ordering::Relaxed)
}

/// 统一的进度条更新函数
#[inline]
pub fn update_progress_bar(
    bar: &Arc<Bar>,
    current_tested: usize,
    success_count: usize,
    total_ips: usize,
) {
    bar.update_all(1, format!("{}/{}", current_tested, total_ips), success_count.to_string());
}