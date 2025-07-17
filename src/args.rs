use std::env;
use std::time::Duration;
use std::collections::HashMap;
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
        
        let map = Self::parse_args_to_map(&args);

        // 布尔参数
        parsed.help = map.contains_key("h") || map.contains_key("help");
        parsed.httping = map.contains_key("httping");
        parsed.disable_download = map.contains_key("dd");
        parsed.test_all = map.contains_key("all4");
        parsed.show_port = map.contains_key("sp");
//        parsed.icmp_ping = map.contains_key("ping");

        // hu 可以有值也可以没有值
        if let Some(hu_opt) = map.get("hu") {
            parsed.httping_urls_flag = true;
            parsed.httping = true;
            parsed.httping_urls = hu_opt.clone().unwrap_or_default();
        }

        // 数值参数
        parsed.ping_times = parse_u16(&map, "t").unwrap_or(parsed.ping_times);
        parsed.test_count = parse_u16(&map, "dn").unwrap_or(parsed.test_count);
        parsed.tcp_port = parse_u16(&map, "tp").unwrap_or(parsed.tcp_port);
        parsed.print_num = parse_u16(&map, "p").unwrap_or(parsed.print_num);
        parsed.max_loss_rate = parse_f32(&map, "tlr").unwrap_or(parsed.max_loss_rate);
        parsed.min_speed = parse_f32(&map, "sl").unwrap_or(parsed.min_speed);
        parsed.target_num = parse_u32(&map, "tn");
        parsed.max_threads = parse_usize(&map, "n").map(|v| v.clamp(1, 1024)).unwrap_or(parsed.max_threads);

        // 时间参数
        parsed.timeout_duration = parse_duration_secs(&map, "dt").map(Some).unwrap_or(parsed.timeout_duration);
        parsed.global_timeout_duration = parse_duration_secs(&map, "timeout");
        parsed.max_delay = parse_duration_millis(&map, "tl").unwrap_or(parsed.max_delay);
        parsed.min_delay = parse_duration_millis(&map, "tll").unwrap_or(parsed.min_delay);

        // 字符串参数
        parsed.url = get_string(&map, "url").unwrap_or_else(|| parsed.url.clone());
        parsed.urlist = get_string(&map, "urlist").unwrap_or_else(|| parsed.urlist.clone());
        parsed.httping_cf_colo = get_string(&map, "colo").unwrap_or_else(|| parsed.httping_cf_colo.clone());
        parsed.ip_file = get_string(&map, "f").unwrap_or_else(|| parsed.ip_file.clone());
        parsed.ip_text = get_string(&map, "ip").unwrap_or_else(|| parsed.ip_text.clone());
        parsed.ip_url = get_string(&map, "ipurl").unwrap_or_else(|| parsed.ip_url.clone());
        parsed.output = get_string(&map, "o").unwrap_or_else(|| parsed.output.clone());

        parsed
    }

    // 将命令行参数解析为HashMap
    fn parse_args_to_map(args: &[String]) -> HashMap<String, Option<String>> {
        let mut map = HashMap::new();
        let mut i = 1;
        while i < args.len() {
            if args[i].starts_with('-') {
                let key = args[i].trim_start_matches('-').to_string();
                let value = if i + 1 < args.len() && !args[i + 1].starts_with('-') {
                    i += 1;
                    Some(args[i].clone())
                } else {
                    None
                };
                map.insert(key, value);
            }
            i += 1;
        }
        map
    }
}

/// 解析u16数值参数
fn parse_u16(map: &HashMap<String, Option<String>>, key: &str) -> Option<u16> {
    map.get(key).and_then(|v| v.as_ref()).and_then(|s| s.parse().ok())
}

/// 解析u32数值参数
fn parse_u32(map: &HashMap<String, Option<String>>, key: &str) -> Option<u32> {
    map.get(key).and_then(|v| v.as_ref()).and_then(|s| s.parse().ok())
}

/// 解析usize数值参数
fn parse_usize(map: &HashMap<String, Option<String>>, key: &str) -> Option<usize> {
    map.get(key).and_then(|v| v.as_ref()).and_then(|s| s.parse().ok())
}

/// 解析f32数值参数
fn parse_f32(map: &HashMap<String, Option<String>>, key: &str) -> Option<f32> {
    map.get(key).and_then(|v| v.as_ref()).and_then(|s| s.parse().ok())
}

/// 解析时间参数（秒）
fn parse_duration_secs(map: &HashMap<String, Option<String>>, key: &str) -> Option<Duration> {
    map.get(key).and_then(|v| v.as_ref()).and_then(|s| s.parse::<u64>().ok().map(Duration::from_secs))
}

/// 解析时间参数（毫秒）
fn parse_duration_millis(map: &HashMap<String, Option<String>>, key: &str) -> Option<Duration> {
    map.get(key).and_then(|v| v.as_ref()).and_then(|s| s.parse::<u64>().ok().map(Duration::from_millis))
}

/// 获取字符串参数
fn get_string(map: &HashMap<String, Option<String>>, key: &str) -> Option<String> {
    map.get(key).and_then(|v| v.as_ref()).cloned()
}

/// 解析并验证参数
pub fn parse_args() -> Args {
    let args = Args::parse();

    // 显示帮助信息并退出
    if args.help {
        print_help();
        std::process::exit(0);
    }

    // 验证IP来源参数
    if args.ip_file.is_empty() && args.ip_url.is_empty() && args.ip_text.is_empty() {
        eprintln!("错误: 必须指定一个或多个IP来源参数 (-f, -ipurl 或 -ip)");
        std::process::exit(1);
    }

    // 验证HTTPing参数
    if args.httping_urls_flag && args.httping_urls.is_empty() && args.url.is_empty() && args.urlist.is_empty() {
        eprintln!("错误: 使用 -hu 参数并且没有传入测速地址时，必须通过 -url 或 -urlist 指定测速地址");
        std::process::exit(1);
    }

    // 验证下载测速参数
    if !args.disable_download && args.url.is_empty() && args.urlist.is_empty() {
        eprintln!("错误: 未设置测速地址，在没有使用 -dd 参数时，请使用 -url 或 -urlist 参数指定下载测速的测速地址");
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
