use std::net::SocketAddr;
use std::time::Duration;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use reqwest::{Client, Response};
use crate::args::Args;
use crate::progress::Bar;
use crate::ip::{IpBuffer, load_ip_to_buffer};

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
pub async fn build_reqwest_client(addr: SocketAddr, host: &str) -> Option<Client> {
    let client = Client::builder()
        .resolve(host, addr) // 解析域名
        .connect_timeout(Duration::from_millis(2000)) // 连接超时
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
    if timeout_flag.load(Ordering::SeqCst) {true} else {false}
}