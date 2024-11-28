use std::net::IpAddr;
use std::time::Duration;
use std::cmp::Ordering;
use clap::ArgMatches;
use thiserror::Error;
use tokio::sync::AcquireError;

#[derive(Error, Debug)]
pub enum SpeedTestError {
    #[error("{0}")]
    Error(String),
}

pub type SpeedTestResult<T> = std::result::Result<T, SpeedTestError>;

#[derive(Debug, Clone)]
pub struct Config {
    pub routines: u32,          // 延迟测速线程数
    pub ping_times: u32,        // 延迟测速次数
    pub test_count: u32,        // 下载测速数量
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

impl Config {
    fn validate_routines(r: u32) -> u32 {
        if r == 0 {
            1
        } else if r > 1000 {
            1000
        } else {
            r
        }
    }

    pub fn from_matches(matches: &ArgMatches) -> SpeedTestResult<Self> {
        let mut config = Self::default();  // 先获取默认值

        // 使用 get_one 获取用户输入的值，如果没有就使用默认值
        if let Some(n) = matches.get_one::<u32>("n") {
            config.routines = Self::validate_routines(*n);
        }
        if let Some(t) = matches.get_one::<u32>("t") {
            config.ping_times = *t;
        }
        if let Some(dn) = matches.get_one::<u32>("dn") {
            config.test_count = *dn;
        }
        if let Some(dt) = matches.get_one::<u64>("dt") {
            config.download_time = Duration::from_secs(*dt);
        }
        if let Some(tp) = matches.get_one::<u16>("tp") {
            config.tcp_port = *tp;
        }
        if let Some(url) = matches.get_one::<String>("url") {
            config.url = url.clone();
        }
        if let Some(code) = matches.get_one::<u16>("httping-code") {
            config.httping_status_code = *code;
        }
        if let Some(colo) = matches.get_one::<String>("cfcolo") {
            config.httping_cf_colo = colo.clone();
        }
        if let Some(tl) = matches.get_one::<u64>("tl") {
            config.max_delay = Duration::from_millis(*tl);
        }
        if let Some(tll) = matches.get_one::<u64>("tll") {
            config.min_delay = Duration::from_millis(*tll);
        }
        if let Some(tlr) = matches.get_one::<f32>("tlr") {
            config.max_loss_rate = *tlr;
        }
        if let Some(sl) = matches.get_one::<f64>("sl") {
            config.min_speed = *sl;
        }
        if let Some(p) = matches.get_one::<u32>("p") {
            config.print_num = *p;
        }
        if let Some(f) = matches.get_one::<String>("f") {
            config.ip_file = f.clone();
        }
        if let Some(ip) = matches.get_one::<String>("ip") {
            config.ip_text = ip.clone();
        }
        if let Some(o) = matches.get_one::<String>("o") {
            config.output = o.clone();
        }

        config.httping = matches.contains_id("httping");
        config.disable_download = matches.contains_id("dd");
        config.test_all = matches.contains_id("all4");

        // 处理 IPv6 测试模式
        if matches.contains_id("more6") {
            config.ipv6_amount = Some(262144); // 2^18
        } else if matches.contains_id("lots6") {
            config.ipv6_amount = Some(65536);  // 2^16
        } else if matches.contains_id("many6") {
            config.ipv6_amount = Some(4096);   // 2^12
        } else if matches.contains_id("some6") {
            config.ipv6_amount = Some(256);    // 2^8
        }

        // 处理 IPv4 测试数量
        if matches.contains_id("many4") {
            config.ipv4_amount = Some(4096);   // 2^12
        }

        // 处理自定义测试数量
        if let Some(v4) = matches.get_one::<String>("v4") {
            let amount = parse_test_amount(v4, true);
            config.ipv4_amount = Some(amount);
        }

        if let Some(v6) = matches.get_one::<String>("v6") {
            let amount = parse_test_amount(v6, false);
            config.ipv6_amount = Some(amount);
        }

        // 处理 IPv6 测试模式
        if matches.contains_id("more6") {
            config.ipv6_num_mode = Some("more".to_string());
        } else if matches.contains_id("lots6") {
            config.ipv6_num_mode = Some("lots".to_string());
        } else if matches.contains_id("many6") {
            config.ipv6_num_mode = Some("many".to_string());
        } else if matches.contains_id("some6") {
            config.ipv6_num_mode = Some("some".to_string());
        }

        Ok(config)
    }

    pub fn is_test_all(&self) -> bool {
        self.test_all
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            routines: 200,          // 默认200线程
            ping_times: 4,          // 默认测试4次
            test_count: 10,         // 默认下载测试10个
            download_time: Duration::from_secs(10),  // 默认10秒
            tcp_port: 443,          // 默认443端口
            url: String::from("https://cf.xiu2.xyz/url"),  // 默认URL
            httping: false,
            httping_status_code: 200,
            httping_cf_colo: String::new(),
            max_delay: Duration::from_millis(9999),
            min_delay: Duration::from_millis(0),
            max_loss_rate: 1.0,
            min_speed: 0.0,
            print_num: 10,
            ip_file: String::from("ip.txt"),
            ip_text: String::new(),
            output: String::from("result.csv"),
            disable_download: false,
            test_all: false,
            ipv4_amount: None,
            ipv6_amount: None,
            ipv6_num_mode: None,
        }
    }
}

// 修改为 trait 实现
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
        SpeedTestError::Error(format!("线程控制失败: {}", err))
    }
} 

// 解析测试数量表达式 (2^n±m)，并进行自适应范围处理
pub fn parse_test_amount(expr: &str, is_v4: bool) -> u32 {
    let max_amount = if is_v4 { 
        2u32.pow(16) // IPv4 最大值 2^16
    } else {
        2u32.pow(20) // IPv6 最大值 2^20
    };

    let amount = {
        let expr = expr.replace(" ", "");
        if let Some(plus_pos) = expr.find('+') {
            // 处理加法
            let base = expr[..plus_pos].parse::<u32>().unwrap_or(0);
            let add = expr[plus_pos+1..].parse::<u32>().unwrap_or(0);
            2u32.pow(base) + add
        } else if let Some(minus_pos) = expr.find('-') {
            // 处理减法
            let base = expr[..minus_pos].parse::<u32>().unwrap_or(0);
            let sub = expr[minus_pos+1..].parse::<u32>().unwrap_or(0);
            2u32.pow(base).saturating_sub(sub)
        } else {
            // 单个数字
            expr.parse::<u32>().unwrap_or(0)
        }
    };

    // 自适应范围处理
    if amount <= 0 {
        1 // 小于等于0时取1
    } else if amount > max_amount {
        max_amount // 超过最大限制时取最大值
    } else {
        amount
    }
} 

impl From<std::io::Error> for SpeedTestError {
    fn from(err: std::io::Error) -> Self {
        SpeedTestError::Error(err.to_string())
    }
}

impl From<reqwest::Error> for SpeedTestError {
    fn from(err: reqwest::Error) -> Self {
        SpeedTestError::Error(err.to_string()) 
    }
}

impl From<csv::Error> for SpeedTestError {
    fn from(err: csv::Error) -> Self {
        SpeedTestError::Error(err.to_string())
    }
} 