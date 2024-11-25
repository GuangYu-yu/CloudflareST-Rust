use std::net::IpAddr;
use std::time::Duration;
use std::cmp::Ordering;
use anyhow::Context;
use clap::ArgMatches;
use thiserror::Error;
use tokio::sync::AcquireError;

#[derive(Error, Debug)]
pub enum SpeedTestError {
    #[error("IP parse error: {0}")]
    IPParseError(String),
    
    #[error("Network error: {0}")]
    NetworkError(#[from] std::io::Error),
    
    #[error("HTTP error: {0}")]
    HttpError(#[from] reqwest::Error),
    
    #[error("Invalid configuration: {0}")]
    ConfigError(String),
    
    #[error("Download error: {0}")]
    DownloadError(String),
    
    #[error("CSV write error: {0}")]
    CsvError(#[from] csv::Error),

    #[error("Other error: {0}")]
    Other(#[from] anyhow::Error),

    #[error("Semaphore acquire error: {0}")]
    AcquireError(String),
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
}

impl CloudflareIPData {
    pub fn new(ping_data: PingData) -> Self {
        let loss_rate = ping_data.loss_rate();
        Self {
            ping_data,
            loss_rate,
            download_speed: 0.0,
            config: Config::default(),
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
        let config = Self {
            routines: Self::validate_routines(*matches.get_one::<u32>("n")
                .context("Missing n parameter")?),
            ping_times: *matches.get_one::<u32>("t")
                .context("Missing t parameter")?,
            test_count: *matches.get_one::<u32>("dn")
                .context("Missing dn parameter")?,
            download_time: Duration::from_secs(
                *matches.get_one::<u64>("dt")
                    .context("Missing dt parameter")?
            ),
            tcp_port: *matches.get_one::<u16>("tp")
                .context("Missing tp parameter")?,
            url: matches.get_one::<String>("url")
                .map(|s| s.to_string())
                .unwrap_or_else(|| "https://cf.xiu2.xyz/url".to_string()),
            httping: matches.contains_id("httping"),
            httping_status_code: *matches.get_one::<u16>("httping-code")
                .context("Missing httping-code parameter")?,
            httping_cf_colo: matches.get_one::<String>("cfcolo")
                .map(|s| s.to_string())
                .unwrap_or_default(),
            max_delay: Duration::from_millis(
                *matches.get_one::<u64>("tl")
                    .context("Missing tl parameter")?
            ),
            min_delay: Duration::from_millis(
                *matches.get_one::<u64>("tll")
                    .context("Missing tll parameter")?
            ),
            max_loss_rate: *matches.get_one::<f32>("tlr")
                .context("Missing tlr parameter")?,
            min_speed: *matches.get_one::<f64>("sl")
                .context("Missing sl parameter")?,
            print_num: *matches.get_one::<u32>("p")
                .context("Missing p parameter")?,
            ip_file: matches.get_one::<String>("f")
                .map(|s| s.to_string())
                .unwrap_or_else(|| "ip.txt".to_string()),
            ip_text: matches.get_one::<String>("ip")
                .map(|s| s.to_string())
                .unwrap_or_default(),
            output: matches.get_one::<String>("o")
                .map(|s| s.to_string())
                .unwrap_or_else(|| "result.csv".to_string()),
            disable_download: matches.contains_id("dd"),
            test_all: matches.contains_id("allip"),
        };
        
        validate_config(&config)?;
        Ok(config)
    }

    pub fn is_test_all(&self) -> bool {
        self.test_all
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            routines: 200,
            ping_times: 4,
            test_count: 10,
            download_time: Duration::from_secs(10),
            tcp_port: 443,
            url: String::from("https://cf.xiu2.xyz/url"),
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

impl From<std::net::AddrParseError> for SpeedTestError {
    fn from(err: std::net::AddrParseError) -> Self {
        SpeedTestError::IPParseError(err.to_string())
    }
}

// 在配置验证失败时使用
pub fn validate_config(config: &Config) -> Result<(), SpeedTestError> {
    if config.tcp_port == 0 {
        return Err(SpeedTestError::ConfigError("Invalid TCP port".into()));
    }
    Ok(())
}

// 在下载失败时使用
pub fn handle_download_error(err: std::io::Error) -> SpeedTestError {
    SpeedTestError::DownloadError(err.to_string())
}

impl From<AcquireError> for SpeedTestError {
    fn from(err: AcquireError) -> Self {
        SpeedTestError::AcquireError(err.to_string())
    }
} 