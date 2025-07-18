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
#[derive(Clone, Debug)]
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

/// 初始化 Ping 测试的基本参数
pub fn init_ping_test(args: &Args) -> (Arc<Mutex<IpBuffer>>, Arc<Mutex<PingDelaySet>>, Arc<Bar>) {
    // 加载 IP 缓冲区
    let ip_buffer = load_ip_to_buffer(args);

    // 获取预计总 IP 数量用于进度条
    let total_expected = ip_buffer.total_expected();
    
    // 转换为线程安全的形式
    let ip_buffer_arc = Arc::new(Mutex::new(ip_buffer));
    
    // 创建进度条，使用正确的格式
    let bar = Arc::new(Bar::new(total_expected as u64, "可用:", ""));
    
    (
        ip_buffer_arc,
        Arc::new(Mutex::new(Vec::new())),
        bar
    )
}

/// Ping 初始化
pub fn create_base_ping(args: &Args, timeout_flag: Arc<AtomicBool>) -> BasePing {
    let (ip_buffer, csv, bar) = init_ping_test(args);
    
    BasePing::new(
        ip_buffer,
        csv,
        bar,
        Arc::new(args.clone()),
        Arc::new(AtomicUsize::new(0)),
        timeout_flag,
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
    // 检查IP缓冲区是否为空
    let available_ips = {
        let ip_buffer_guard = base.ip_buffer.lock().unwrap();
        if ip_buffer_guard.is_empty() && ip_buffer_guard.total_expected() == 0 {
            return Ok(Vec::new());
        }
        ip_buffer_guard.total_expected()
    };

    // 使用FuturesUnordered来动态管理任务
    let mut tasks = FuturesUnordered::new();
    
    // 根据实际IP数量和线程池并发级别计算初始任务数
    let pool_concurrency = crate::pool::GLOBAL_POOL.get().unwrap().max_threads;
    let initial_tasks = available_ips.min(pool_concurrency).max(1);
    
    // 初始填充任务队列
    for _ in 0..initial_tasks {
        let addr = {
            let mut ip_buffer_guard = base.ip_buffer.lock().unwrap();
            ip_buffer_guard.pop()
        };
        
        if let Some(addr) = addr {
            let task = handler_factory.create_handler(addr);
            tasks.push(tokio::spawn(async move {
                task.await;
                Ok::<(), io::Error>(())
            }));
        }
    }
    
    // 动态处理任务完成和添加新任务
    while let Some(result) = tasks.next().await {
        // 检查是否收到超时信号
        if check_timeout_signal(&base.timeout_flag) {
            break;
        }
        
        // 检查是否达到目标成功数量
        if let Some(target_num) = base.args.target_num {
            if base.success_count.load(Ordering::Relaxed) >= target_num as usize {
                break;
            }
        }
        
        // 处理已完成的任务
        let _ = result;
        
        // 添加新任务
        let addr = {
            let mut ip_buffer_guard = base.ip_buffer.lock().unwrap();
            ip_buffer_guard.pop()
        };
        
        if let Some(addr) = addr {
            let task = handler_factory.create_handler(addr);
            tasks.push(tokio::spawn(async move {
                task.await;
                Ok::<(), io::Error>(())
            }));
        }
    }

    // 更新进度条为完成状态
    base.bar.done();

    // 收集所有测试结果
    let mut results = {
        let mut csv_guard = base.csv.lock().unwrap();
        std::mem::take(&mut *csv_guard)
    };
    
    // 使用common模块的排序函数
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
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
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
pub fn parse_colo_filters(colo_filter: &str) -> Vec<String> {
    colo_filter
        .split(',')
        .map(|s| s.trim().to_uppercase())
        .filter(|s| !s.is_empty())
        .collect()
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

// 检查数据中心是否匹配过滤条件
pub fn is_colo_matched(data_center: &str, colo_filters: &[String]) -> bool {
    !data_center.is_empty() && 
        (colo_filters.is_empty() || 
         colo_filters.iter().any(|filter| data_center.to_uppercase() == filter.to_uppercase()))
}

/// 排序结果
pub fn sort_results(results: &mut PingDelaySet) {
    // 计算平均值
    let total_count = results.len() as f32;
    let (total_speed, total_loss, total_delay) = results.iter().fold((0.0, 0.0, 0.0), |acc, data| {
        (acc.0 + data.download_speed.unwrap_or(0.0), acc.1 + data.loss_rate(), acc.2 + data.delay)
    });

    let avg_speed = total_speed / total_count;
    let avg_loss = total_loss / total_count;
    let avg_delay = total_delay / total_count;

    // 检查是否有下载速度数据
    let has_download_speed = results.iter().any(|r| r.download_speed.is_some());
    
    // 根据是否有下载速度数据选择权重
    let (speed_weight, delay_weight, loss_weight) = if has_download_speed {
        // 下载测速结果
        (0.5, -0.2, -0.3)
    } else {
        // ping测速结果
        (0.0, -0.4, -0.6)
    };

    // 计算分数并排序
    results.sort_by(|a, b| {
        let calculate_score = |data: &PingData| {
            let speed_diff = data.download_speed.unwrap_or(0.0) - avg_speed;
            let delay_diff = data.delay - avg_delay;
            let loss_diff = data.loss_rate() - avg_loss;
            
            speed_diff * speed_weight + delay_diff * delay_weight + loss_diff * loss_weight
        };

        let a_score = calculate_score(a);
        let b_score = calculate_score(b);

        b_score.partial_cmp(&a_score).unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// 检查是否收到超时信号，如果是则打印信息并返回 true
pub fn check_timeout_signal(timeout_flag: &AtomicBool) -> bool {
    timeout_flag.load(Ordering::SeqCst)
}