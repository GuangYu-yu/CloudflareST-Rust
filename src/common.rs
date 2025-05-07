use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use reqwest::{Client, Response};
use crate::PingResult;
use crate::args::Args;
use crate::progress::Bar;
use prettytable::{Row, Cell};
use crate::ip::{IpBuffer, load_ip_to_buffer};

// 定义浏览器标识常量
pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

// 定义通用的 PingData 结构体
#[derive(Clone, Debug)]
pub struct PingData {
    pub ip: IpAddr,
    pub sent: u16,
    pub received: u16,
    pub delay: f32,
    pub download_speed: Option<f32>,
    pub data_center: String,
}

impl PingData {
    pub fn new(ip: IpAddr, sent: u16, received: u16, delay: f32) -> Self {
        Self {
            ip,
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

/// 从 PingResult 中提取速度、丢包率和延迟信息
pub fn extract_ping_metrics(result: &PingResult) -> (Option<f32>, f32, f32) {
    match result {
        PingResult::Http(data) => (data.download_speed, data.loss_rate(), data.delay),
        PingResult::Tcp(data) => (data.download_speed, data.loss_rate(), data.delay),
        PingResult::Icmp(data) => (data.download_speed, data.loss_rate(), data.delay),
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

/// 构建用于测试的 reqwest 客户端
pub async fn build_reqwest_client(ip: IpAddr, url: &str, port: u16, timeout: Duration) -> Option<Client> {
    // 解析原始URL以获取主机名
    let url_parts = match url::Url::parse(url) {
        Ok(parts) => parts,
        Err(_) => return None,
    };
    
    let host = match url_parts.host_str() {
        Some(host) => host,
        None => return None,
    };
    
    build_reqwest_client_with_host(ip, port, host, timeout.as_millis() as u64).await
}

/// 使用主机名构建 reqwest 客户端
pub async fn build_reqwest_client_with_host(ip: IpAddr, port: u16, host: &str, timeout_ms: u64) -> Option<Client> {
    // 构建客户端，使用 reqwest 内置的 resolve 方法
    let client = Client::builder()
        .resolve(host, SocketAddr::new(ip, port))
        .timeout(Duration::from_millis(timeout_ms))
        .user_agent(USER_AGENT)  // 使用常量
//        .danger_accept_invalid_certs(true)  // 跳过证书验证
        .pool_max_idle_per_host(0) // 禁用连接复用
        .redirect(reqwest::redirect::Policy::none()) // 禁止重定向
        .build();
    
    match client {
        Ok(client) => Some(client),
        Err(_) => None,
    }
}

/// 发送GET请求
pub async fn send_request(client: &Client, url: &str) -> Option<Response> {
    match client.get(url).send().await {
        Ok(resp) => Some(resp),
        Err(_) => None,
    }
}

// 发送GET请求但只获取响应头
pub async fn send_head_request(
    client: &reqwest::Client,
    url: &str,
) -> Option<reqwest::Response> {
    // 使用GET方法，但设置Connection: close头部，并且只读取到头部结束
    let response = client.get(url)
        .header("Connection", "close")
        .header("Accept-Encoding", "identity") // 禁用压缩，便于找到头部结束位置
        .header("Range", "bytes=0-1") // 只请求极少量数据
        .send()
        .await
        .ok()?;
    
    Some(response)
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

/// 将 PingData 转换为 CSV 记录
pub fn ping_data_to_csv_record(data: &PingData) -> Vec<String> {
    vec![
        data.ip.to_string(),
        data.sent.to_string(),
        data.received.to_string(),
        format!("{:.2}", data.loss_rate()),
        format!("{:.2}", data.delay),
        match data.download_speed {
            Some(speed) => format!("{:.2}", speed / 1024.0 / 1024.0),
            None => String::new(),
        },
        data.data_center.clone(),
    ]
}

/// 将 PingData 转换为表格行
pub fn ping_data_to_table_row(data: &PingData) -> Row {
    Row::new(vec![
        Cell::new(&data.ip.to_string()),
        Cell::new(&data.sent.to_string()),
        Cell::new(&data.received.to_string()),
        Cell::new(&format!("{:.2}", data.loss_rate())),
        Cell::new(&format!("{:.2}", data.delay)),
        Cell::new(&match data.download_speed {
            Some(speed) => format!("{:.2}", speed / 1024.0 / 1024.0),
            None => String::new(),
        }),
        Cell::new(&data.data_center),
    ])
}

/// 从 URL 列表或单一 URL 获取测试 URL 列表
pub async fn get_url_list(url: &str, urlist: &str) -> Vec<String> {
    if !urlist.is_empty() {
        // 最多尝试3次
        for i in 1..=3 {
            if let Ok(response) = reqwest::get(urlist).await {
                if let Ok(content) = response.text().await {
                    return content.lines()
                        .map(|line| line.trim())
                        .filter(|line| !line.is_empty() && !line.starts_with("//") && !line.starts_with('#'))
                        .map(|line| line.to_string())
                        .collect();
                }
            }
            
            // 只有在不是最后一次尝试时才打印重试信息和等待
            if i < 3 {
                println!("测速地址列表请求失败，正在重试 ({}/{})", i, 3);
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            } else {
                println!("获取测速地址列表已达到最大重试次数");
            }
        }
    }
    
    // 使用单一URL作为默认值
    vec![url.to_string()]
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

/// 检查并处理下载测速结果，返回是否满足条件
pub fn process_download_result(
    data: &mut PingData,
    speed: Option<f32>,
    maybe_colo: Option<String>,
    min_speed: f32,
    colo_filters: &[String],
) -> bool {
    data.download_speed = speed;
    
    // 如果数据中心为空且获取到了新的数据中心信息，则更新
    if data.data_center.is_empty() {
        if let Some(colo) = maybe_colo {
            data.data_center = colo;
        }
    }
    
    // 检查速度是否符合要求
    let speed_match = match speed {
        Some(s) => s >= min_speed * 1024.0 * 1024.0,
        None => false,  // 如果速度为None，视为不符合要求
    };
    
    // 如果设置了 colo 过滤条件，需要同时满足速度和数据中心要求
    if !colo_filters.is_empty() {
        // 检查数据中心是否符合过滤条件
        let colo_match = !data.data_center.is_empty() && 
            (colo_filters.is_empty() || colo_filters.iter().any(|filter| data.data_center.to_uppercase() == *filter));
        
        // 同时满足速度和数据中心要求
        speed_match && colo_match
    } else {
        // 如果没有设置 colo 过滤条件，只需要满足速度要求
        speed_match
    }
}

/// 按延迟和丢包率排序Ping结果
pub fn sort_ping_results(results: &mut PingDelaySet) {
    // 按延迟和丢包率排序
    results.sort_by(|a, b| {
        a.delay.partial_cmp(&b.delay)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                let a_loss = a.loss_rate();
                let b_loss = b.loss_rate();
                a_loss.partial_cmp(&b_loss).unwrap_or(std::cmp::Ordering::Equal)
            })
    });
}

/// 检查是否收到超时信号，如果是则打印信息并返回 true
pub fn check_timeout_signal(timeout_flag: &AtomicBool) -> bool {
    if timeout_flag.load(Ordering::SeqCst) {true} else {false}
}