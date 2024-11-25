use std::net::IpAddr;
use std::time::Duration;
use anyhow::{Result, Context};
use clap::ArgMatches;
use std::cmp::Ordering;

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

    pub fn compare_delay(&self, other: &Self) -> Ordering {
        self.delay.cmp(&other.delay)
    }
}

#[derive(Debug, Clone)]
pub struct CloudflareIPData {
    pub ping_data: PingData,
    pub loss_rate: f32,
    pub download_speed: f64,
}

impl CloudflareIPData {
    pub fn new(ping_data: PingData) -> Self {
        let loss_rate = ping_data.loss_rate();
        Self {
            ping_data,
            loss_rate,
            download_speed: 0.0,
        }
    }

    pub fn set_download_speed(&mut self, speed: f64) {
        self.download_speed = speed;
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
        if !self.ping_data.ip.is_ipv4() && other.ping_data.ip.is_ipv4() {
            return Ordering::Greater;
        }
        if self.ping_data.ip.is_ipv4() && !other.ping_data.ip.is_ipv4() {
            return Ordering::Less;
        }
        self.download_speed.partial_cmp(&other.download_speed)
            .unwrap_or(Ordering::Equal)
            .reverse()
            .then_with(|| self.ping_data.compare_delay(&other.ping_data))
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
    pub fn from_matches(matches: &ArgMatches) -> Result<Self> {
        Ok(Self {
            routines: matches.value_of("n")
                .context("Missing n parameter")?
                .parse()
                .context("Invalid n parameter")?,
                
            ping_times: matches.value_of("t")
                .context("Missing t parameter")?
                .parse()
                .context("Invalid t parameter")?,
                
            test_count: matches.value_of("dn")
                .context("Missing dn parameter")?
                .parse()
                .context("Invalid dn parameter")?,
                
            download_time: Duration::from_secs(
                matches.value_of("dt")
                    .context("Missing dt parameter")?
                    .parse()
                    .context("Invalid dt parameter")?
            ),
                
            tcp_port: matches.value_of("tp")
                .context("Missing tp parameter")?
                .parse()
                .context("Invalid tp parameter")?,
                
            url: matches.value_of("url")
                .unwrap_or("https://cf.xiu2.xyz/url")
                .to_string(),
                
            httping: matches.is_present("httping"),
            
            httping_status_code: matches.value_of("httping-code")
                .context("Missing httping-code parameter")?
                .parse()
                .context("Invalid httping-code parameter")?,
                
            httping_cf_colo: matches.value_of("cfcolo")
                .unwrap_or("")
                .to_string(),
                
            max_delay: Duration::from_millis(
                matches.value_of("tl")
                    .context("Missing tl parameter")?
                    .parse()
                    .context("Invalid tl parameter")?
            ),
                
            min_delay: Duration::from_millis(
                matches.value_of("tll")
                    .context("Missing tll parameter")?
                    .parse()
                    .context("Invalid tll parameter")?
            ),
                
            max_loss_rate: matches.value_of("tlr")
                .context("Missing tlr parameter")?
                .parse()
                .context("Invalid tlr parameter")?,
                
            min_speed: matches.value_of("sl")
                .context("Missing sl parameter")?
                .parse()
                .context("Invalid sl parameter")?,
                
            print_num: matches.value_of("p")
                .context("Missing p parameter")?
                .parse()
                .context("Invalid p parameter")?,
                
            ip_file: matches.value_of("f")
                .unwrap_or("ip.txt")
                .to_string(),
                
            ip_text: matches.value_of("ip")
                .unwrap_or("")
                .to_string(),
                
            output: matches.value_of("o")
                .unwrap_or("result.csv")
                .to_string(),
                
            disable_download: matches.is_present("dd"),
            test_all: matches.is_present("allip"),
        })
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