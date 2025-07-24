use std::net::SocketAddr;
use std::time::Duration;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering, AtomicUsize};
use std::io;
use reqwest::{Client, Response};
use futures::stream::{FuturesUnordered, StreamExt};
use crate::args::Args;
use crate::progress::Bar;
use crate::ip::{IpBuffer, load_ip_to_buffer};
use std::future::Future;
use std::pin::Pin;

// 定义浏览器标识常量
pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

// 定义通用的 PingData 结构体
#[derive(Clone)]
pub struct PingData {
    pub addr: SocketAddr,
    pub sent: u16,
    pub received: u16,
    pub delay: f32,
    pub download_speed: Option<f32>,
    pub data_center: String,
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
    pub ip_buffer: Arc<Mutex<IpBuffer>>,
    pub csv: Arc<Mutex<PingDelaySet>>,
    pub bar: Arc<Bar>,
    pub args: Arc<Args>,
    pub success_count: Arc<AtomicUsize>,
    pub timeout_flag: Arc<AtomicBool>,
}

impl BasePing {
    pub fn new(
        ip_buffer: Arc<Mutex<IpBuffer>>,
        csv: Arc<Mutex<PingDelaySet>>,
        bar: Arc<Bar>,
        args: Arc<Args>,
        success_count: Arc<AtomicUsize>,
        timeout_flag: Arc<AtomicBool>,
    ) -> Self {
        Self {
            ip_buffer,
            csv,
            bar,
            args,
            success_count,
            timeout_flag,
        }
    }

    pub fn clone_shared_state(&self) -> (Arc<Mutex<PingDelaySet>>, Arc<Bar>, Arc<Args>, Arc<AtomicUsize>) {
        (
            Arc::clone(&self.csv),
            Arc::clone(&self.bar),
            Arc::clone(&self.args),
            Arc::clone(&self.success_count),
        )
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

/// 构建 Reqwest 客户端
pub async fn build_reqwest_client(addr: SocketAddr, host: &str, timeout_ms: u64) -> Option<Client> {
    let client = Client::builder()
        .resolve(host, addr) // 解析域名
        .timeout(Duration::from_millis(timeout_ms)) // 整个请求超时时间
        .user_agent(USER_AGENT)  // 使用常量
//        .danger_accept_invalid_certs(true)  // 跳过证书验证
//        .pool_max_idle_per_host(0) // 禁用连接复用
        .redirect(reqwest::redirect::Policy::none()) // 禁止重定向
        .build()
        .ok();
    
    client
}

/// 从响应中提取数据中心信息
pub fn extract_data_center(resp: &Response) -> Option<String> {
    resp.headers().get("cf-ray")?
        .to_str()
        .ok()?
        .rsplit('-')
        .next()
        .map(str::to_owned)
}

/// Ping 初始化
pub fn create_base_ping(args: &Args, timeout_flag: Arc<AtomicBool>) -> BasePing {
    // 加载 IP 缓冲区
    let ip_buffer = load_ip_to_buffer(args);

    // 获取预计总 IP 数量用于进度条
    let total_expected = ip_buffer.total_expected();

    // 创建 BasePing 所需各项资源并初始化
    BasePing::new(
        Arc::new(Mutex::new(ip_buffer)),                  // 转换为线程安全的 IP 缓冲区
        Arc::new(Mutex::new(Vec::new())),                 // 空的 PingDelaySet，用于记录延迟
        Arc::new(Bar::new(total_expected as u64, "可用:", "")), // 创建进度条
        Arc::new(args.clone()),                           // 参数包装
        Arc::new(AtomicUsize::new(0)),                    // 成功计数器
        timeout_flag,                                     // 提前中止标记
    )
}

pub trait HandlerFactory: Send + Sync + 'static {
    fn create_handler(&self, addr: SocketAddr) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
}

/// 通用的 ping 测试运行函数
pub async fn run_ping_test(
    base: &BasePing,
    handler_factory: Arc<dyn HandlerFactory>,
) -> Result<PingDelaySet, io::Error> {
    // 获取锁检查是否还有IP可用
    let mut ip_buffer_guard = base.ip_buffer.lock().unwrap();

    if ip_buffer_guard.is_empty() {
        // 生产者已停止且缓冲为空，没有IP可测，直接返回空结果
        return Ok(Vec::new());
    }

    // 线程池最大并发数量
    let pool_concurrency = crate::pool::GLOBAL_POOL.get().unwrap().max_threads;

    // 创建异步任务管理器
    let mut tasks = FuturesUnordered::new();

    // 批量初始启动任务
    for _ in 0..pool_concurrency {
        if let Some(addr) = ip_buffer_guard.pop() {
            let task = handler_factory.create_handler(addr);
            tasks.push(tokio::spawn(async move {
                task.await;
                Ok::<(), io::Error>(())
            }));
        } else {
            break; // 没有更多IP
        }
    }

    drop(ip_buffer_guard); // 释放锁

    // 动态循环处理任务，直到超时或任务耗尽
    while let Some(result) = tasks.next().await {
        // 检查超时信号或是否达到目标成功数量，满足任一条件则提前退出
        if check_timeout_signal(&base.timeout_flag)
            || base.args.target_num
                .map(|tn| base.success_count.load(Ordering::Relaxed) >= tn as usize)
                .unwrap_or(false)
        {
            break;
        }

        // 处理已完成的任务结果（这里忽略错误）
        let _ = result;

        // 继续添加新任务
        let mut ip_buffer_guard = base.ip_buffer.lock().unwrap();
        if let Some(addr) = ip_buffer_guard.pop() {
            let task = handler_factory.create_handler(addr);
            tasks.push(tokio::spawn(async move {
                task.await;
                Ok::<(), io::Error>(())
            }));
        }
        drop(ip_buffer_guard);
    }

    // 所有任务结束，更新进度条
    base.bar.done();

    // 收集和排序结果
    let mut csv_guard = base.csv.lock().unwrap();
    let mut results = std::mem::take(&mut *csv_guard);
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
        if let Some(response) = reqwest::get(url).await.ok() {
            if let Ok(content) = response.text().await {
                return content.lines()
                    .map(|line| line.trim())
                    .filter(|line| !line.is_empty() && !line.starts_with("//") && !line.starts_with('#'))
                    .map(|line| line.to_string())
                    .collect();
            }
        }
        
        // 只有在不是最后一次尝试时才打印重试信息和等待
        if i < max_retries {
            println!("列表请求失败，正在重试 ({}/{})", i, max_retries);
            tokio::time::sleep(Duration::from_secs(1)).await;
        } else {
            println!("获取列表已达到最大重试次数");
        }
    }
    
    Vec::new()
}

/// 从 URL 列表或单一 URL 获取测试 URL 列表
pub async fn get_url_list(url: &str, urlist: &str) -> Vec<String> {
    if !urlist.is_empty() {
        let list = get_list(urlist, 3).await;
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
pub fn parse_colo_filters(colo_filter: &str) -> Vec<Arc<str>> {
    colo_filter
        .split(',')
        .map(|s| s.trim().to_uppercase())
        .filter(|s| !s.is_empty())
        .map(|s| s.into())
        .collect()
}

// 检查数据中心是否匹配过滤条件
pub fn is_colo_matched(data_center: &str, colo_filters: &[Arc<str>]) -> bool {
    !data_center.is_empty() && 
    (colo_filters.is_empty() || 
     colo_filters.iter().any(|filter| filter.as_ref() == data_center.to_uppercase()))
}

/// 判断测试结果是否符合筛选条件
pub fn should_keep_result(data: &PingData, args: &Args) -> bool {
    // 检查丢包率
    if data.loss_rate() > args.max_loss_rate {
        return false;
    }
    
    // 检查延迟上下限
    if data.delay < args.min_delay.as_millis() as f32 ||
       data.delay > args.max_delay.as_millis() as f32 {
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

    // 先算平均值
    let total_count = results.len() as f32;
    let (total_speed, total_loss, total_delay) = results.iter().fold((0.0, 0.0, 0.0), |acc, d| {
        (acc.0 + d.download_speed.unwrap_or(0.0), acc.1 + d.loss_rate(), acc.2 + d.delay)
    });
    let avg_speed = total_speed / total_count;
    let avg_loss = total_loss / total_count;
    let avg_delay = total_delay / total_count;

    let has_speed = results.iter().any(|r| r.download_speed.is_some());

    // 计算分数
    let score = |d: &PingData| {
        let speed_diff = d.download_speed.unwrap_or(0.0) - avg_speed;
        let loss_diff = d.loss_rate() - avg_loss;
        let delay_diff = d.delay - avg_delay;

        if has_speed {
            speed_diff * 0.5 + delay_diff * -0.2 + loss_diff * -0.3
        } else {
            delay_diff * -0.4 + loss_diff * -0.6
        }
    };

    results.sort_unstable_by(|a, b| {
        score(b).partial_cmp(&score(a)).unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// 检查是否收到超时信号，如果是则打印信息并返回 true
pub fn check_timeout_signal(timeout_flag: &AtomicBool) -> bool {
    timeout_flag.load(Ordering::Relaxed)
}