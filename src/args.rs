use std::env;
use std::time::Duration;
use colored::*;

#[derive(Clone)]
pub struct Args {
    // 延迟测速相关
//    pub icmp_ping: bool,        // 是否使用ICMP Ping测速
    pub ping_times: u16,        // 延迟测速次数
    pub tcp_port: u16,           // 指定测速端口
    pub url: String,             // 指定测速地址
    pub urlist: String,          // 指定测速地址列表
    
    // HTTP测速相关
    pub httping: bool,           // 是否使用HTTP测速
    pub httping_cf_colo: String, // 匹配指定地区
    pub httping_urls: String,    // 指定Httping模式的测速地址
    pub httping_urls_flag: bool, // 是否使用-hu参数（无论是否有值）
    
    // 延迟过滤相关
    pub max_delay: Duration,     // 平均延迟上限
    pub min_delay: Duration,     // 平均延迟下限
    pub max_loss_rate: f32,      // 丢包几率上限
    
    // 下载测速相关
    pub test_count: u16,       // 下载测速数量
    pub timeout_duration: Option<Duration>, // 下载测速时间
    pub min_speed: f32,          // 下载速度下限
    pub disable_download: bool,  // 是否禁用下载测速
    
    // 输出相关
    pub target_num: Option<u32>,  // 当Ping到指定可用数量时提前结束Ping
    pub print_num: u16,        // 显示结果数量
    pub ip_file: String,         // IP段数据文件
    pub ip_text: String,         // 指定IP段数据
    pub ip_url: String,          // 从URL获取IP段数据
    pub output: String,          // 输出结果文件
    
    // 其他选项
    pub test_all: bool,          // 是否测速全部IP
    pub help: bool,              // 显示帮助
    pub show_port: bool,         // 是否显示端口号
    
    pub global_timeout_duration: Option<Duration>, // 全局超时时间
    pub max_threads: usize,      // 最大线程数
}

impl Args {
    pub fn new() -> Self {
        Self {
//            icmp_ping: false,
            ping_times: 4,
            tcp_port: 443,
            url: String::new(),
            urlist: String::new(),
            
            httping: false,
            httping_cf_colo: String::new(),
            httping_urls: String::new(),
            httping_urls_flag: false,
            
            max_delay: Duration::from_millis(2000),
            min_delay: Duration::from_millis(0),
            max_loss_rate: 1.0,
            
            test_count: 10,
            timeout_duration: Some(Duration::from_secs(10)),
            min_speed: 0.0,
            disable_download: false,
            target_num: None,
            
            print_num: 10,
            ip_file: String::new(),
            ip_text: String::new(),
            ip_url: String::new(),
            output: "result.csv".to_string(),
            
            test_all: false,
            help: false,
            show_port: false,
            
            global_timeout_duration: None,
            max_threads: 256,
        }
    }

    pub fn parse() -> Self {
        let args: Vec<String> = env::args().collect();
        let mut parsed = Self::new();
        
        // 将参数重组为参数组（可能是单参数或参数+值）
        let mut arg_groups: Vec<Vec<String>> = Vec::new();
        let mut i = 1; // 跳过程序名
        
        while i < args.len() {
            let arg = &args[i];
            
            // 确保是参数标志
            if arg.starts_with('-') {
                let mut group = vec![arg.clone()];
                
                // 检查下一个参数是否是值（不以'-'开头）
                if i + 1 < args.len() && !args[i + 1].starts_with('-') {
                    group.push(args[i + 1].clone());
                    i += 2; // 跳过参数和值
                } else {
                    i += 1; // 只跳过参数
                }
                
                arg_groups.push(group);
            } else {
                // 跳过非参数标志
                i += 1;
            }
        }
        
        // 处理重组后的参数组
        for group in arg_groups {
            let name = group[0].trim_start_matches('-').to_string();
            let value = if group.len() > 1 { Some(&group[1]) } else { None };
            
            Self::handle_arg(&name, value, &mut parsed);
        }

        parsed
    }
    
    // 统一处理所有参数
    fn handle_arg(name: &str, value: Option<&String>, parsed: &mut Self) {
        match name {
            // 无值标志参数
            "h" | "help" => parsed.help = true,
            "httping" => parsed.httping = true,
    //      "ping" => parsed.icmp_ping = true,
            "dd" => parsed.disable_download = true,
            "all4" => parsed.test_all = true,
            "sp" => parsed.show_port = true,
            
            // -hu 参数可以有值也可以没有值
            "hu" => {
                parsed.httping_urls_flag = true;
                parsed.httping = true; // 设置httping为true
                Self::set_string(value, &mut parsed.httping_urls);
            },
            
            // 带值参数
            "t" => Self::set_if_valid::<u16>(value, &mut parsed.ping_times),
            "dn" => Self::set_if_valid::<u16>(value, &mut parsed.test_count),
            "dt" => Self::set_duration_secs(value, &mut parsed.timeout_duration),
            "tp" => Self::set_if_valid::<u16>(value, &mut parsed.tcp_port),
            "url" => Self::set_string(value, &mut parsed.url),
            "urlist" => Self::set_string(value, &mut parsed.urlist),
            "colo" => Self::set_string(value, &mut parsed.httping_cf_colo),
            "tl" => Self::set_duration_ms(value, &mut parsed.max_delay),
            "tll" => Self::set_duration_ms(value, &mut parsed.min_delay),
            "tlr" => Self::set_if_valid::<f32>(value, &mut parsed.max_loss_rate),
            "sl" => Self::set_if_valid::<f32>(value, &mut parsed.min_speed),
            "p" => Self::set_if_valid::<u16>(value, &mut parsed.print_num),
            "n" => Self::set_with_clamp::<usize>(value, &mut parsed.max_threads, 5, 2048),
            "f" => Self::set_string(value, &mut parsed.ip_file),
            "ip" => Self::set_string(value, &mut parsed.ip_text),
            "ipurl" => Self::set_string(value, &mut parsed.ip_url),
            "o" => Self::set_string(value, &mut parsed.output),
            "timeout" => Self::set_duration_secs(value, &mut parsed.global_timeout_duration),
            "tn" => Self::set_option::<u32>(value, &mut parsed.target_num),
            _ => { print_help(); eprintln!("错误: 不支持的参数: {}", name); std::process::exit(1); }
        }
    }
    
    // 解析字符串值为指定类型，如果解析失败则返回 None
    fn parse_value<T: std::str::FromStr>(value: Option<&String>) -> Option<T> {
        value.and_then(|val| val.parse::<T>().ok())
    }
    
    // 解析字符串值并设置目标字段，如果解析失败则保持原值
    fn set_if_valid<T: std::str::FromStr>(value: Option<&String>, target: &mut T) {
        if let Some(parsed) = Self::parse_value::<T>(value) {
            *target = parsed;
        }
    }
    
    // 解析字符串值为 Duration (毫秒)
    fn set_duration_ms(value: Option<&String>, target: &mut Duration) {
        if let Some(ms) = Self::parse_value::<u64>(value) {
            *target = Duration::from_millis(ms);
        }
    }
    
    // 解析字符串值为 Duration (秒)
    fn set_duration_secs(value: Option<&String>, target: &mut Option<Duration>) {
        if let Some(secs) = Self::parse_value::<u64>(value) {
            *target = Some(Duration::from_secs(secs));
        }
    }
    
    // 解析字符串值并应用范围限制
    fn set_with_clamp<T>(value: Option<&String>, target: &mut T, min: T, max: T) 
    where T: std::str::FromStr + Ord + Copy {
        if let Some(parsed) = Self::parse_value::<T>(value) {
            *target = parsed.clamp(min, max);
        }
    }
    
    // 设置字符串值
    fn set_string(value: Option<&String>, target: &mut String) {
        if let Some(val) = value {
            *target = val.clone();
        }
    }
    
    // 设置可选数值
    fn set_option<T: std::str::FromStr>(value: Option<&String>, target: &mut Option<T>) {
        if let Some(parsed) = Self::parse_value::<T>(value) {
            *target = Some(parsed);
        }
    }
}

macro_rules! print_arg {
    ($name:expr, $desc:expr, $default:expr) => {
        println!("  {:<10}   {}{}", $name.green(), $desc, $default.dimmed());
    };
}

pub fn print_help() {
    println!("{}", "# CloudflareST-Rust".bold().blue());
    
    // 基本参数
    println!("\n{}:", "基本参数".bold());
    print_arg!("-url", "TLS 模式的 Httping 或下载测速所使用的测速地址（https://example.com/file）", "[默认：未指定]");
    print_arg!("-urlist", "从 URL 内读取测速地址列表（https://example.com/url_list.txt）", "[默认：未指定]");
    print_arg!("-f", "从文件或文件路径读取 IP 或 CIDR ", "[默认：未指定]");
    print_arg!("-ip", "直接指定 IP 或 CIDR（多个用逗号分隔）", "[默认：未指定]");
    print_arg!("-ipurl", "从URL读取 IP 或 CIDR （https://example.com/ip_list.txt) ", "[默认：未指定]");
    print_arg!("-timeout", "程序超时退出时间（秒）", "[默认：不限制]");
    
    // 测速参数
    println!("\n{}:", "测速参数".bold());
    print_arg!("-t", "延迟测速次数 ", "[默认：4]");
    print_arg!("-dn", "所需下载测速结果数量 ", "[默认：10]");
    print_arg!("-dt", "下载测速时间（秒）", "[默认：10]");
    print_arg!("-tp", "测速端口 ", "[默认：443]");
    print_arg!("-all4", "测速全部 IPv4 ", "[默认：否]");
    print_arg!("-tn", "当 Ping 到指定可用数量，提前结束 Ping ", "[默认：否]");
    
    // 测速选项
    println!("\n{}:", "测速选项".bold());
    print_arg!("-httping", "使用非 TLS 模式的 Httping ，无需测速地址 ", "[默认：否]");
//    print_arg!("-ping", "ICMP-Ping 测速模式 ", "[默认：否]");
    print_arg!("-dd", "禁用下载测速 ", "[默认：否]");
    print_arg!("-hu", "使用 TLS 模式的 Httping ，可指定其 URL 测速地址或使用-url 或 -urlist 指定 ", "[默认：否]");
    print_arg!("-colo", "匹配指定地区（示例：HKG,SJC）", "[默认：未指定]");
    print_arg!("-n", "延迟测速的线程数量 ", "[默认：128]");
    
    // 结果参数
    println!("\n{}:", "结果参数".bold());
    print_arg!("-tl", "延迟上限（毫秒）", "[默认：2000]");
    print_arg!("-tll", "延迟下限（毫秒）", "[默认：0]");
    print_arg!("-tlr", "丢包率上限 ", "[默认：1.00]");
    print_arg!("-sl", "下载速度下限（MB/s）", "[默认：0.00]");
    print_arg!("-p", "终端显示结果数量 ", "[默认：10]");
    print_arg!("-sp", "启用结果的端口号显示 ", "[默认：否]");
    print_arg!("-o", "输出结果文件（文件名或文件路径）", "[默认：result.csv]");
}

pub fn parse_args() -> Args {
    let args = Args::parse();
    
    if args.help {
        print_help();
        std::process::exit(0);
    }
    
    // 检查IP来源参数是否至少指定了一个
    if args.ip_file.is_empty() && args.ip_url.is_empty() && args.ip_text.is_empty() {
        println!("错误: 必须指定一个或多个IP来源参数 (-f, -ipurl 或 -ip)");
        std::process::exit(1);
    }

    // 检查-hu参数：如果使用了-hu但没有提供URL，也没有设置-url或-urlist
    if args.httping_urls_flag && args.httping_urls.is_empty() && args.url.is_empty() && args.urlist.is_empty() {
        println!("错误: 使用 -hu 参数并且没有传入测速地址时，必须通过 -url 或 -urlist 指定测速地址");
        std::process::exit(1);
    }
    
    // 检查下载测速地址（当没有使用-dd时）
    if !args.disable_download && args.url.is_empty() && args.urlist.is_empty() {
        println!("错误: 未设置测速地址，在没有使用 -dd 参数时，请使用 -url 或 -urlist 参数指定下载测速的测速地址");
        std::process::exit(1);
    }
    
    args
}