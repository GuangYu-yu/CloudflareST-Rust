use std::net::IpAddr;
use std::time::Duration;
use std::cmp::Ordering;
use thiserror::Error;
use tokio::sync::AcquireError;

#[derive(Error, Debug)]
pub enum SpeedTestError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Request error: {0}")]
    RequestError(#[from] reqwest::Error),

    #[error("CSV error: {0}")]
    CsvError(#[from] csv::Error),

    #[error("Thread control error: {0}")]
    ThreadError(String),
}

#[derive(Debug, Clone)]
pub struct Config {
    pub ping_times: u32,          // 延迟测速次数
    pub test_count: u32,         // 下载测速数量
    pub download_time: Duration, // 下载测速时间
    pub tcp_port: u16,          // 测速端口
    pub url: String,            // 测速URL
    
    pub httping: bool,                // 是否使用HTTP测速
    pub httping_status_code: u16,     // HTTP状态码
    pub httping_cf_colo: String,      // 匹配指定地区
    
    pub max_delay: Duration,    // 平均延迟上限
    pub min_delay: Duration,    // 平均延迟下限
    pub max_loss_rate: f32,     // 丢包率上限
    pub min_speed: f64,         // 下载速度下限
    
    pub print_num: u32,         // 显示结果数量
    pub ip_file: String,        // IP段数据文件
    pub ip_text: String,        // 指定IP段数据
    pub output: String,         // 输出文件
    
    pub disable_download: bool, // 禁用下载测速
    pub test_all: bool,        // 测试全部IP
    pub ipv4_amount: Option<u32>,  // IPv4 测试数量
    pub ipv6_amount: Option<u32>,  // IPv6 测试数量
    pub ipv6_num_mode: Option<String>, // IPv6 数量模式
    pub ipv4_num_mode: Option<String>, // IPv4 数量模式
    pub max_ip_count: usize,  // 添加 IP 总量上限参数
}

impl Config {
    pub fn is_test_all(&self) -> bool {
        self.test_all
    }
}

#[derive(Debug, Clone)]
pub struct PingData {
    pub ip: IpAddr,
    pub sended: u32,
    pub received: u32,
    pub delay: Duration,
}

impl PingData {
    pub fn new(ip: IpAddr, sended: u32, received: u32, delay: Duration) -> Self {
        Self {
            ip,
            sended,
            received,
            delay,
        }
    }

    pub fn loss_rate(&self) -> f32 {
        if self.sended == 0 {
            return 1.0;
        }
        (self.sended - self.received) as f32 / self.sended as f32
    }
}

#[derive(Debug, Clone)]
pub struct CloudflareIPData {
    pub ping_data: PingData,
    pub loss_rate: f32,
    pub download_speed: f64,
    pub config: Config,
    pub colo: String,
}

impl CloudflareIPData {
    pub fn new(ping_data: PingData) -> Self {
        let loss_rate = ping_data.loss_rate();
        Self {
            ping_data,
            loss_rate,
            download_speed: 0.0,
            config: Config::default(),
            colo: String::new(),
        }
    }

    pub fn to_string_vec(&self) -> Vec<String> {
        vec![
            self.ping_data.ip.to_string(),
            self.ping_data.sended.to_string(),
            self.ping_data.received.to_string(),
            format!("{:.2}", self.loss_rate),
            format!("{:.2}", self.ping_data.delay.as_secs_f64() * 1000.0),
            format!("{:.2}", self.download_speed / 1024.0 / 1024.0),
            self.colo.clone(),
        ]
    }
}

// 实现排序特性
impl Ord for CloudflareIPData {
    fn cmp(&self, other: &Self) -> Ordering {
        let self_rate = self.loss_rate;
        let other_rate = other.loss_rate;
        
        if self_rate != other_rate {
            self_rate.partial_cmp(&other_rate)
                .unwrap_or(Ordering::Equal)
        } else {
            self.ping_data.delay.cmp(&other.ping_data.delay)
        }
    }
}

impl PartialOrd for CloudflareIPData {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for CloudflareIPData {
    fn eq(&self, other: &Self) -> bool {
        self.download_speed == other.download_speed &&
        self.ping_data.delay == other.ping_data.delay
    }
}

impl Eq for CloudflareIPData {}

pub type PingDelaySet = Vec<CloudflareIPData>;
pub type DownloadSpeedSet = Vec<CloudflareIPData>;

pub trait DelayFilter {
    fn filter_delay(self) -> Self;
    fn filter_loss_rate(self) -> Self;
}

impl DelayFilter for PingDelaySet {
    fn filter_delay(self) -> Self {
        unsafe {
            if INPUT_MAX_DELAY > MAX_DELAY || INPUT_MIN_DELAY < MIN_DELAY {
                return self;
            }
            if INPUT_MAX_DELAY == MAX_DELAY && INPUT_MIN_DELAY == MIN_DELAY {
                return self;
            }
            
            let mut filtered = Vec::new();
            for ip_data in self {
                if ip_data.ping_data.delay > INPUT_MAX_DELAY {
                    break;
                }
                if ip_data.ping_data.delay < INPUT_MIN_DELAY {
                    continue;
                }
                filtered.push(ip_data);
            }
            filtered
        }
    }

    fn filter_loss_rate(self) -> Self {
        unsafe {
            if INPUT_MAX_LOSS_RATE >= MAX_LOSS_RATE {
                return self;
            }
            
            let mut filtered = Vec::new();
            for ip_data in self {
                if ip_data.loss_rate > INPUT_MAX_LOSS_RATE {
                    break;
                }
                filtered.push(ip_data);
            }
            filtered
        }
    }
}

// 这些常量和静态变量应该保留在 types.rs 中
pub const MAX_DELAY: Duration = Duration::from_millis(9999);
pub const MIN_DELAY: Duration = Duration::ZERO;
pub const MAX_LOSS_RATE: f32 = 1.0;

// 修改静态变量命名并添加 unsafe 块
pub static mut INPUT_MAX_DELAY: Duration = MAX_DELAY;
pub static mut INPUT_MIN_DELAY: Duration = MIN_DELAY;
pub static mut INPUT_MAX_LOSS_RATE: f32 = MAX_LOSS_RATE;

impl From<AcquireError> for SpeedTestError {
    fn from(err: AcquireError) -> Self {
        SpeedTestError::ThreadError(format!("线程控制失败: {}", err))
    }
} 

impl Default for Config {
    fn default() -> Self {
        Self {
            ping_times: 4,          // -t 4
            test_count: 10,         // -dn 10
            download_time: Duration::from_secs(10),  // -dt 10
            tcp_port: 443,          // -tp 443
            url: String::from("https://cf.xiu2.xyz/url"),  // -url
            httping: false,         // -httping
            httping_status_code: 200,  // -httping-code
            httping_cf_colo: String::new(),  // -cfcolo (默认空)
            max_delay: Duration::from_millis(9999),  // -tl 9999
            min_delay: Duration::from_millis(0),     // -tll 0
            max_loss_rate: 1.0,     // -tlr 1.00
            min_speed: 0.0,         // -sl 0.00
            print_num: 10,          // -p 10
            ip_file: String::from("ip.txt"),  // -f ip.txt
            ip_text: String::new(),  // -ip (默认空)
            output: String::from("result.csv"),  // -o result.csv
            disable_download: false,  // -dd (默认启用)
            test_all: false,         // -all4 (默认否)
            ipv4_amount: None,       // -v4 (默认无)
            ipv6_amount: None,       // -v6 (默认无)
            ipv6_num_mode: None,     // -more6/-lots6/-many6/-some6 (默认无)
            ipv4_num_mode: None,     // -many4 (默认无)
            max_ip_count: 500_000,  // 默认50万
        }
    }
}

// 解析测试数量，只接受单个数字
pub fn parse_test_amount(expr: &str, is_v4: bool) -> u32 {
    let max_amount = if is_v4 { 
        2u32.pow(16) // IPv4 最大值 2^16
    } else {
        2u32.pow(20) // IPv6 最大值 2^20
    };

    // 直接解析数字，解析失败返回0
    let amount = expr.parse::<u32>().unwrap_or(0);
    
    // 确保不超过最大值
    amount.min(max_amount)
}