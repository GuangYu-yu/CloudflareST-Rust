use std::env;
use std::time::Duration;
use colored::*;

#[derive(Clone)]
pub struct Args {
    // 延迟测速相关
    pub ping_times: u16,       // 延迟测速次数
    pub tcp_port: u16,           // 指定测速端口
    pub url: String,             // 指定测速地址
    pub urlist: String,          // 指定测速地址列表
    
    // HTTP测速相关
    pub httping: bool,           // 是否使用HTTP测速
    pub httping_status_code: Option<u16>, // HTTP有效状态码，None表示接受200、301、302
    pub httping_cf_colo: String, // 匹配指定地区
    
    // 延迟过滤相关
    pub max_delay: Duration,     // 平均延迟上限
    pub min_delay: Duration,     // 平均延迟下限
    pub max_loss_rate: f32,      // 丢包几率上限
    
    // 下载测速相关
    pub test_count: u16,       // 下载测速数量
    pub timeout: String,         // 下载测速时间(字符串)
    pub timeout_duration: Option<Duration>, // 下载测速时间
    pub min_speed: f32,          // 下载速度下限
    pub disable_download: bool,  // 是否禁用下载测速
    
    // 输出相关
    pub print_num: u16,        // 显示结果数量
    pub ip_file: String,         // IP段数据文件
    pub ip_text: String,         // 指定IP段数据
    pub ip_url: String,          // 从URL获取IP段数据
    pub output: String,          // 输出结果文件
    
    // 其他选项
    pub test_all: bool,          // 是否测速全部IP
    pub help: bool,              // 显示帮助
    
    // 全局超时
    pub global_timeout: String,         // 全局超时时间(字符串)
    pub global_timeout_duration: Option<Duration>, // 全局超时时间
}

impl Args {
    pub fn new() -> Self {
        Self {
            ping_times: 4,
            tcp_port: 443,
            url: String::new(),
            urlist: String::new(),
            
            httping: false,
            httping_status_code: None,
            httping_cf_colo: String::new(),
            
            max_delay: Duration::from_millis(2000),
            min_delay: Duration::from_millis(0),
            max_loss_rate: 1.0,
            
            test_count: 10,
            timeout: "10s".to_string(),
            timeout_duration: Some(Duration::from_secs(10)),
            min_speed: 0.0,
            disable_download: false,
            
            print_num: 10,
            ip_file: "ip.txt".to_string(),
            ip_text: String::new(),
            ip_url: String::new(),
            output: "result.csv".to_string(),
            
            test_all: false,
            help: false,
            
            global_timeout: String::new(),
            global_timeout_duration: None,
        }
    }

    pub fn parse() -> Self {
        let args: Vec<String> = env::args().collect();
        let mut parsed = Self::new();
        let mut i = 1;  // 跳过程序名
        
        while i < args.len() {
            let arg = &args[i];
            
            // 确保是参数标志，统一处理单破折号和双破折号
            if !arg.starts_with('-') {
                i += 1;
                continue;
            }

            // 去除所有前导破折号
            let name = arg.trim_start_matches('-').to_string();
            
            // 检查是否是无值标志参数
            match name.as_str() {
                "h" | "help" => {
                    parsed.help = true;
                    i += 1;
                    continue;
                },
                "httping" => {
                    parsed.httping = true;
                    i += 1;
                    continue;
                },
                "dd" => {
                    parsed.disable_download = true;
                    i += 1;
                    continue;
                },
                "all4" => {
                    parsed.test_all = true;
                    i += 1;
                    continue;
                },
                _ => {}
            }
            
            // 处理带值的参数
            if i + 1 < args.len() && !args[i + 1].starts_with('-') {
                match name.as_str() {
                    "t" => {
                        if let Ok(val) = args[i + 1].parse::<u16>() {
                            parsed.ping_times = val;
                        }
                    },
                    "dn" => {
                        if let Ok(val) = args[i + 1].parse::<u16>() {
                            parsed.test_count = val;
                        }
                    },
                    "dt" => {
                        parsed.timeout = args[i + 1].clone();
                        if let Ok(val) = args[i + 1].parse::<u64>() {
                            parsed.timeout_duration = Some(Duration::from_secs(val));
                        }
                    },
                    "tp" => {
                        if let Ok(val) = args[i + 1].parse::<u16>() {
                            parsed.tcp_port = val;
                        }
                    },
                    "url" => {
                        parsed.url = args[i + 1].clone();
                    },
                    "urlist" => {
                        parsed.urlist = args[i + 1].clone();
                    },
                    "hc" => {
                        if let Ok(val) = args[i + 1].parse::<u16>() {
                            parsed.httping_status_code = Some(val);
                        }
                    },
                    "colo" => {
                        parsed.httping_cf_colo = args[i + 1].clone();
                    },
                    "tl" => {
                        if let Ok(val) = args[i + 1].parse::<u64>() {
                            parsed.max_delay = Duration::from_millis(val);
                        }
                    },
                    "tll" => {
                        if let Ok(val) = args[i + 1].parse::<u64>() {
                            parsed.min_delay = Duration::from_millis(val);
                        }
                    },
                    "tlr" => {
                        if let Ok(val) = args[i + 1].parse::<f32>() {
                            parsed.max_loss_rate = val;
                        }
                    },
                    "sl" => {
                        if let Ok(val) = args[i + 1].parse::<f32>() {
                            parsed.min_speed = val;
                        }
                    },
                    "p" => {
                        if let Ok(val) = args[i + 1].parse::<u16>() {
                            parsed.print_num = val;
                        }
                    },
                    "f" => {
                        parsed.ip_file = args[i + 1].clone();
                    },
                    "ip" => {
                        parsed.ip_text = args[i + 1].clone();
                    },
                    "ipurl" => {
                        parsed.ip_url = args[i + 1].clone();
                    },
                    "o" => {
                        parsed.output = args[i + 1].clone();
                    },
                    "timeout" => {
                        parsed.global_timeout = args[i + 1].clone();
                        // 解析超时时间
                        parsed.global_timeout_duration = parse_duration(&args[i + 1]);
                    },
                    _ => {}
                }
                i += 2;  // 跳过参数名和值
            } else {
                i += 1;
            }
        }

        parsed
    }
    

}

// 解析时间字符串为Duration
fn parse_duration(duration_str: &str) -> Option<Duration> {
    // 如果是空字符串，表示不限制时间
    if duration_str.is_empty() {
        return None;
    }
    
    // 尝试使用humantime库解析时间字符串
    match humantime::parse_duration(duration_str) {
        Ok(duration) => Some(duration),
        Err(err) => {
            println!("解析超时时间失败: {}，将不限制运行时间", err);
            None
        }
    }
}

macro_rules! print_arg {
    ($name:expr, $desc:expr, $default:expr) => {
        println!("  {:<10}   {}{}", $name.green(), $desc, $default.dimmed());
    };
}

pub fn print_help() {
    println!("{}", "CloudflareST-Rust".bold().blue());
    
    // 基本参数
    println!("\n{}:", "基本参数".bold());
    print_arg!("-url", "测速地址 (https://example.com/file)", "[默认: 未指定]");
    print_arg!("-urlist", "从 URL 内读取测速地址列表 (https://example.com/url_list.txt)", "[默认: 未指定]");
    print_arg!("-f", "从文件或文件路径读取 IP 或 CIDR", "[默认: ip.txt]");
    print_arg!("-ip", "直接指定 IP 或 CIDR (多个用逗号分隔)", "[默认: 未指定]");
    print_arg!("-ipurl", "从URL读取 IP 或 CIDR (https://example.com/ip_list.txt)", "[默认: 未指定]");
    print_arg!("-o", "输出结果文件（文件名或文件路径）", "[默认: result.csv]");
    print_arg!("-h", "打印帮助说明", "[默认: 否]");
    print_arg!("-timeout", "程序超时退出时间（示例：1h3m6s）", "[默认: 不限制]");
    
    // 测速参数
    println!("\n{}:", "测速参数".bold());
    print_arg!("-t", "延迟测速次数", "[默认: 4]");
    print_arg!("-dn", "所需下载测速结果数量", "[默认: 10]");
    print_arg!("-dt", "下载测速时间（秒）", "[默认: 10]");
    print_arg!("-tp", "测速端口", "[默认: 443]");
    print_arg!("-dd", "禁用下载测速", "[默认: 否]");
    print_arg!("-all4", "测速全部IPv4", "[默认: 否]");
    
    // HTTP测速选项
    println!("\n{}:", "HTTP测速选项".bold());
    print_arg!("-httping", "Httping模式", "[默认: 否]");
    print_arg!("-hc", "有效状态码", "[默认: 接受200/301/302]");
    print_arg!("-colo", "匹配指定地区（示例：HKG,SJC）", "[默认: 未指定]");
    
    // 筛选参数
    println!("\n{}:", "筛选参数".bold());
    print_arg!("-tl", "延迟上限（毫秒）", "[默认: 2000]");
    print_arg!("-tll", "延迟下限（毫秒）", "[默认: 0]");
    print_arg!("-tlr", "丢包率上限", "[默认: 1.00]");
    print_arg!("-sl", "下载速度下限（MB/s）", "[默认: 0.00]");
    print_arg!("-p", "终端显示结果数量", "[默认: 10]");
}

pub fn parse_args() -> Args {
    let args = Args::parse();
    
    if args.help {
        print_help();
        std::process::exit(0);
    }
    
    // 检查测速地址是否为空（当需要下载测速或使用HTTP测速时）
    if args.url.is_empty() && args.urlist.is_empty() && (!args.disable_download || args.httping) {
        println!("错误: 未设置测速地址，在使用 -httping 或没有使用 -dd 参数时，请使用 -url 或 -urlist 参数指定测速地址");
        println!("{}", "使用 -h 参数查看帮助".red());
        std::process::exit(1);
    }
    
    args
}
