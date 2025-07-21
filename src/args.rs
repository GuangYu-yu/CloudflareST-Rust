use std::env;
use std::time::Duration;
use colored::*;  // 用于终端彩色输出

/// 命令行参数配置结构体
#[derive(Clone)]
pub struct Args {
    // 网络测试参数
//    pub icmp_ping: bool,                  // 是否使用ICMP Ping测速
    pub ping_times: u16,                  // Ping测试次数
    pub tcp_port: u16,                    // TCP端口号
    pub url: String,                      // 单个测速URL
    pub urlist: String,                   // URL列表文件路径
    pub httping: bool,                    // 是否启用HTTPing测试
    pub httping_cf_colo: String,          // 指定Cloudflare地区代码
    pub httping_urls: String,             // HTTPing使用的URL列表
    pub httping_urls_flag: bool,          // 是否使用自定义HTTPing URL标志
    pub max_delay: Duration,              // 最大可接受延迟
    pub min_delay: Duration,              // 最小可接受延迟
    pub max_loss_rate: f32,               // 最大丢包率阈值
    pub test_count: u16,                  // 下载测试次数
    pub timeout_duration: Option<Duration>, // 单次测试超时时间
    pub min_speed: f32,                   // 最低下载速度要求(MB/s)
    pub disable_download: bool,           // 是否禁用下载测试
    
    // 结果处理参数
    pub target_num: Option<u32>,          // 提前结束测试的目标数量
    pub print_num: u16,                   // 显示结果数量
    pub ip_file: String,                  // IP列表文件路径
    pub ip_text: String,                  // 直接指定的IP列表
    pub ip_url: String,                   // 获取IP的URL地址
    pub output: String,                   // 结果输出文件
    
    // 功能开关
    pub test_all: bool,                   // 是否测试所有IP
    pub help: bool,                       // 是否显示帮助信息
    pub show_port: bool,                  // 是否在结果中显示端口
    
    // 高级设置
    pub global_timeout_duration: Option<Duration>, // 全局超时设置
    pub max_threads: usize,               // 最大线程数
}

impl Args {
    /// 创建默认参数配置
    pub fn new() -> Self {
        Self {
//            icmp_ping: false,
            ping_times: 4,                        // 默认Ping测试4次
            tcp_port: 443,                       // 默认使用443端口
            url: String::new(),
            urlist: String::new(),
            httping: false,
            httping_cf_colo: String::new(),
            httping_urls: String::new(),
            httping_urls_flag: false,
            max_delay: Duration::from_millis(2000), // 默认最大延迟2000ms
            min_delay: Duration::from_millis(0),  // 默认最小延迟0ms
            max_loss_rate: 1.0,                   // 默认最大丢包率100%
            test_count: 10,                       // 默认下载测试10次
            timeout_duration: Some(Duration::from_secs(10)), // 默认单次超时10秒
            min_speed: 0.0,                       // 默认最低速度0MB/s
            disable_download: false,
            target_num: None,
            print_num: 10,                        // 默认显示前10个结果
            ip_file: String::new(),
            ip_text: String::new(),
            ip_url: String::new(),
            output: "result.csv".to_string(),      // 默认输出文件
            test_all: false,
            help: false,
            show_port: false,
            global_timeout_duration: None,         // 默认无全局超时
            max_threads: 256,                     // 默认最大线程数256
        }
    }

    /// 解析命令行参数
    pub fn parse() -> Self {
        let args: Vec<String> = env::args().collect();
        let mut parsed = Self::new();
        let vec = Self::parse_args_to_vec(&args);

        for (k, v_opt) in vec {
            match k.as_str() {
                // 布尔参数
                "h" | "help" => parsed.help = true,
                "httping" => parsed.httping = true,
                "dd" => parsed.disable_download = true,
                "all4" => parsed.test_all = true,
                "sp" => parsed.show_port = true,
                // "ping" => parsed.icmp_ping = true,

                // hu 可以有值也可以没有值
                "hu" => {
                    parsed.httping_urls_flag = true;
                    parsed.httping = true;
                    parsed.httping_urls = v_opt.unwrap_or_default();
                }

                // 数值参数
                "t"   => if let Some(v) = v_opt.and_then(|s| s.parse::<u16>().ok()) { parsed.ping_times = v.clamp(1, u16::MAX); }
                "dn"  => if let Some(v) = v_opt.and_then(|s| s.parse::<u16>().ok()) { parsed.test_count = v.clamp(1, u16::MAX); }
                "tp"  => if let Some(v) = v_opt.and_then(|s| s.parse::<u16>().ok()) { parsed.tcp_port = v.clamp(1, u16::MAX); }
                "p"   => if let Some(v) = v_opt.and_then(|s| s.parse::<u16>().ok()) { parsed.print_num = v.clamp(1, u16::MAX); }
                "tlr" => if let Some(v) = v_opt.and_then(|s| s.parse::<f32>().ok()) { parsed.max_loss_rate = v.clamp(0.0, 1.0); }
                "sl"  => if let Some(v) = v_opt.and_then(|s| s.parse::<f32>().ok()) { parsed.min_speed = v.clamp(0.0, f32::MAX); }
                "tn" => parsed.target_num = v_opt.and_then(|s| s.parse().ok()),
                "n"   => if let Some(v) = v_opt.and_then(|s| s.parse::<usize>().ok()) { parsed.max_threads = v.clamp(1, 1024); }

                // 时间参数
                "dt"      => if let Some(v) = v_opt.and_then(|s| s.parse::<u64>().ok()) { parsed.timeout_duration = Some(Duration::from_secs(v.clamp(1, 120))); }
                "timeout" => if let Some(v) = v_opt.and_then(|s| s.parse::<u64>().ok()) { parsed.global_timeout_duration = Some(Duration::from_secs(v.clamp(1, 36000))); }
                "tl"  => if let Some(v) = v_opt.and_then(|s| s.parse::<u64>().ok()) { parsed.max_delay = Duration::from_millis(v.clamp(0, 2000)); },
                "tll" => if let Some(v) = v_opt.and_then(|s| s.parse::<u64>().ok()) { parsed.min_delay = Duration::from_millis(v.clamp(0, parsed.max_delay.as_millis().min(u64::MAX as u128) as u64)); },

                // 字符串参数
                "url" => parsed.url = v_opt.unwrap_or_else(|| parsed.url.clone()),
                "urlist" => parsed.urlist = v_opt.unwrap_or_else(|| parsed.urlist.clone()),
                "colo" => parsed.httping_cf_colo = v_opt.unwrap_or_else(|| parsed.httping_cf_colo.clone()),
                "f" => parsed.ip_file = v_opt.unwrap_or_else(|| parsed.ip_file.clone()),
                "ip" => parsed.ip_text = v_opt.unwrap_or_else(|| parsed.ip_text.clone()),
                "ipurl" => parsed.ip_url = v_opt.unwrap_or_else(|| parsed.ip_url.clone()),
                "o" => parsed.output = v_opt.unwrap_or_else(|| parsed.output.clone()),

                // 无效参数：打印错误并退出
                _ => {
                    print_help();
                    println!("{}", format!("无效的参数: {k}").red().bold());
                    std::process::exit(1);
                }
            }
        }

        parsed
    }

    // 解析命令行
    fn parse_args_to_vec(args: &[String]) -> Vec<(String, Option<String>)> {
        let mut vec = Vec::new();
        let mut iter = args.iter().skip(1).peekable();

        while let Some(arg) = iter.next() {
            if arg.starts_with('-') {
                let key = arg.trim_start_matches('-').to_string();
                let value = if let Some(next) = iter.peek() {
                    if !next.starts_with('-') {
                        Some(iter.next().unwrap().clone())
                    } else {
                        None
                    }
                } else {
                    None
                };
                vec.push((key, value));
            }
        }
        vec
    }
}

/// 解析并验证参数
pub fn parse_args() -> Args {
    let args = Args::parse();

    // 显示帮助信息并退出
    if args.help {
        print_help();
        std::process::exit(0);
    }

    // 验证文件是否存在
    if !args.ip_file.is_empty() && !std::path::Path::new(&args.ip_file).exists() {
        eprintln!("{}", format!("错误: 指定的文件不存在").red().bold());
        std::process::exit(1);
    }

    // 验证IP来源参数
    if args.ip_file.is_empty() && args.ip_url.is_empty() && args.ip_text.is_empty() {
        eprintln!("{}", "错误: 必须指定一个或多个IP来源参数 (-f, -ipurl 或 -ip)".red().bold());
        std::process::exit(1);
    }

    // 验证HTTPing参数
    if args.httping_urls_flag && args.httping_urls.is_empty() && args.url.is_empty() && args.urlist.is_empty() {
        eprintln!("{}", "错误: 使用 -hu 参数并且没有传入测速地址时，必须通过 -url 或 -urlist 指定测速地址".red().bold());
        std::process::exit(1);
    }

    // 验证下载测速参数
    if !args.disable_download && args.url.is_empty() && args.urlist.is_empty() {
        eprintln!("{}", "错误: 未设置测速地址，在没有使用 -dd 参数时，请使用 -url 或 -urlist 参数指定下载测速的测速地址".red().bold());
        std::process::exit(1);
    }

    args
}

/// 宏：格式化输出参数帮助信息
macro_rules! print_arg {
    ($name:expr, $desc:expr, $default:expr) => {
        println!("  {:<10}   {}{}", $name.green(), $desc, $default.dimmed());
    };
}

/// 打印帮助信息
pub fn print_help() {
    println!("{}", "# CloudflareST-Rust".bold().blue());
    println!("\n{}:", "基本参数".bold());
    print_arg!("-url", "TLS 模式测速地址", "[默认：未指定]");
    print_arg!("-urlist", "URL 地址列表", "[默认：未指定]");
    print_arg!("-f", "从文件读取 IP", "[默认：未指定]");
    print_arg!("-ip", "直接指定 IP", "[默认：未指定]");
    print_arg!("-ipurl", "从URL读取 IP", "[默认：未指定]");
    print_arg!("-timeout", "全局超时（秒）", "[默认：不限制]");

    println!("\n{}:", "测速参数".bold());
    print_arg!("-t", "延迟测速次数", "[默认：4]");
    print_arg!("-dn", "下载测速数量", "[默认：10]");
    print_arg!("-dt", "下载测速时间（秒）", "[默认：10]");
    print_arg!("-tp", "测速端口", "[默认：443]");
    print_arg!("-all4", "测速全部IPv4", "[默认：否]");
    print_arg!("-tn", "提前结束Ping数量", "[默认：否]");

    println!("\n{}:", "测速选项".bold());
//    print_arg!("-ping", "ICMP-Ping 测速模式 ", "[默认：否]");
    print_arg!("-httping", "非 TLS Httping", "[默认：否]");
    print_arg!("-dd", "禁用下载测速", "[默认：否]");
    print_arg!("-hu", "TLS Httping 可选 URL", "[默认：否]");
    print_arg!("-colo", "指定地区（HKG,SJC）", "[默认：未指定]");
    print_arg!("-n", "线程数量", "[默认：256]");

    println!("\n{}:", "结果参数".bold());
    print_arg!("-tl", "延迟上限（毫秒）", "[默认：2000]");
    print_arg!("-tll", "延迟下限（毫秒）", "[默认：0]");
    print_arg!("-tlr", "丢包率上限", "[默认：1.00]");
    print_arg!("-sl", "下载速度下限（MB/s）", "[默认：0.00]");
    print_arg!("-p", "显示结果数量", "[默认：10]");
    print_arg!("-sp", "显示端口号", "[默认：否]");
    print_arg!("-o", "输出文件", "[默认：result.csv]");
}
