use crate::args::Args;
use crate::ip::{IpBuffer, load_ip_to_buffer};
use crate::progress::Bar;
use crate::hyper::client_builder;
use crate::pool::GLOBAL_LIMITER;
use futures::stream::{FuturesUnordered, StreamExt};
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use crate::warning_println;
use hyper::Uri;
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

pub type PingDelaySet = Vec<PingData>;

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

/*
/// 创建基础客户端构建器
fn client_builder() -> reqwest::ClientBuilder {
    Client::builder()
        // 使用用户标识常量
        .user_agent(USER_AGENT)
}

/// 构建 Reqwest 客户端
pub async fn build_reqwest_client(
    addr: SocketAddr,
    host: &str,
    timeout_ms: u64,
    interface: Option<&str>,
    interface_ips: Option<&InterfaceIps>,
) -> Option<Client> {
    let mut builder = client_builder()
        .resolve(host, addr) // 解析域名
        .timeout(Duration::from_millis(timeout_ms)) // 整个请求超时时间
        //        .danger_accept_invalid_certs(true)         // 跳过证书验证
        //        .pool_max_idle_per_host(0)                 // 禁用连接复用
        .redirect(reqwest::redirect::Policy::none()); // 禁止重定向

    // 如果 interface_ips 不为空，所有平台都根据目标 IP 类型选择源 IP
    if let Some(ips) = interface_ips {
        // 根据 resolve 的 addr 类型选择源 IP
        let source_ip = match addr.ip() {
            IpAddr::V4(_) => ips.ipv4,
            IpAddr::V6(_) => ips.ipv6,
        };

        if let Some(ip) = source_ip {
            builder = builder.local_address(Some(ip));
        }
    } else if let Some(intf) = interface {
        // 如果 interface_ips 为空，但 interface 不为空
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            // Linux 和 macOS 使用接口名
            builder = builder.interface(intf);
        }
        #[cfg(target_os = "windows")]
        {
            // Windows 系统：接口名已经在 args.rs 中处理过，并转换为 IP 地址
            // 这里不需要额外处理，因为 interface_ips 应该已经包含 IP 地址
            let _ = intf; // 占位使用变量以避免警告
        }
    }

    let client = builder.build().ok();
    client
}
 */

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
        Arc::new(Bar::new(total_expected as u64, "可用:", "")), // 创建进度条
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

/// 通用的 ping 测试运行函数
pub async fn run_ping_test(
    base: &BasePing,
    handler_factory: Arc<dyn HandlerFactory>,
) -> Result<PingDelaySet, io::Error> {
    // 并发限制器最大并发数量
    let pool_concurrency = GLOBAL_LIMITER.get().unwrap().max_concurrent;

    // 创建异步任务管理器
    let mut tasks = FuturesUnordered::new();

    // 创建并推送任务
    let create_task = |addr: SocketAddr| handler_factory.create_handler(addr);

    // 用于收集结果
    let mut results = Vec::new();

    // 批量初始启动任务
    for _ in 0..pool_concurrency {
        if let Some(addr) = base.ip_buffer.pop() {
            tasks.push(create_task(addr));
        } else {
            break; // 没有更多IP
        }
    }

    // 动态循环处理任务，直到超时或任务耗尽
    while let Some(result) = tasks.next().await {
        // 检查超时信号或是否达到目标成功数量，满足任一条件则提前退出
        if check_timeout_signal(&base.timeout_flag)
            || base
                .args
                .target_num
                .map(|tn| base.success_count.load(Ordering::Relaxed) >= tn as usize)
                .unwrap_or(false)
        {
            break;
        }

        // 处理任务结果
        if let Some(ping_data) = result {
            // 应用筛选条件
            if should_keep_result(&ping_data, &base.args) {
                // 增加成功计数
                base.success_count.fetch_add(1, Ordering::Relaxed);
                results.push(ping_data);
            }
        }

        // 更新测试计数
        let current_tested = base.tested_count.fetch_add(1, Ordering::Relaxed) + 1;

        // 更新进度条
        let total_ips = base.ip_buffer.total_expected();
        update_progress_bar(
            &base.bar,
            current_tested,
            &base.success_count,
            total_ips,
            None,
        );

        // 继续添加新任务
        if let Some(addr) = base.ip_buffer.pop() {
            tasks.push(create_task(addr));
        }
    }

    // 完成进度条但保持当前进度
    base.bar.done();

    // 排序结果
    sort_results(&mut results);

    Ok(results)
}

/// 从URL获取列表
pub async fn get_list(url: &str, max_retries: u8) -> Vec<String> {
    if url.is_empty() {
        return Vec::new();
    }

    // 最多尝试指定次数
    for i in 1..=max_retries {
        // 创建客户端
        let mut client = match client_builder() {
            Ok(c) => c,
            Err(_) => continue,
        };

        // 构造 URI
        let uri: Uri = match url.parse() {
            Ok(u) => u,
            Err(_) => continue,
        };

        // 发送 GET 请求
        if let Ok(body_bytes) = send_get_request_simple(&mut client, uri.clone(), 5000).await {
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
                .any(|filter| filter == &data_center.to_uppercase()))
}

/// 判断测试结果是否符合筛选条件
pub fn should_keep_result(data: &PingData, args: &Args) -> bool {
    let data_ref = data.as_ref();

    // 检查丢包率
    if data_ref.loss_rate() > args.max_loss_rate {
        return false;
    }

    // 检查延迟上下限
    if data_ref.delay < args.min_delay.as_millis() as f32
        || data_ref.delay > args.max_delay.as_millis() as f32
    {
        return false;
    }

    // 通过所有筛选条件
    true
}

/// 排序结果
pub fn sort_results(results: &mut PingDelaySet) {
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
        let speed_diff = d_ref.download_speed.unwrap_or(0.0) - avg_speed;
        let loss_diff = d_ref.loss_rate() - avg_loss;
        let delay_diff = d_ref.delay - avg_delay;

        if has_speed {
            speed_diff * 0.5 + delay_diff * -0.2 + loss_diff * -0.3
        } else {
            delay_diff * -0.4 + loss_diff * -0.6
        }
    };

    results.sort_unstable_by(|a, b| {
        score(b)
            .partial_cmp(&score(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// 检查是否收到超时信号，如果是则打印信息并返回 true
pub fn check_timeout_signal(timeout_flag: &AtomicBool) -> bool {
    timeout_flag.load(Ordering::Relaxed)
}

/// 统一的进度条更新函数
pub fn update_progress_bar(
    bar: &Arc<Bar>,
    current_tested: usize,
    success_count: &Arc<AtomicUsize>,
    total_ips: usize,
    success_count_override: Option<usize>,
) {
    let current_success =
        success_count_override.unwrap_or_else(|| success_count.load(Ordering::Relaxed));
    bar.grow(1, format!("{}/{}", current_tested, total_ips));
    bar.set_suffix(current_success.to_string());
}