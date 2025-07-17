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
        let args: Vec<String> = env::args().collect(); // 获取原始命令行参数
        let mut parsed = Self::new();                // 创建默认配置
        
        // 遍历所有参数
        let mut i = 1;  // 跳过程序名（索引0）
        while i < args.len() {
            let arg = &args[i];
            if arg.starts_with('-') {
                // 处理参数组（参数名+可能的值）
                let mut group = vec![arg.trim_start_matches('-').to_string()];
                // 检查下一个参数是否是当前参数的值（不以'-'开头）
                if i + 1 < args.len() && !args[i + 1].starts_with('-') {
                    group.push(args[i + 1].clone());
                    i += 2;  // 跳过已处理的值
                } else {
                    i += 1;   // 仅处理参数名
                }
                let name = group[0].clone();
                let value = if group.len() > 1 { Some(&group[1]) } else { None };
                // 将参数映射到配置结构体
                Self::handle_arg(&name, value, &mut parsed);
            } else {
                i += 1;  // 跳过非参数项
            }
        }
        parsed
    }

    /// 处理单个命令行参数
    pub fn handle_arg(name: &str, value: Option<&String>, parsed: &mut Self) {
        match name {
            // 帮助信息开关
            "h" | "help" => parsed.help = true,
            // 功能开关类参数
    //      "ping" => parsed.icmp_ping = true,
            "httping" => parsed.httping = true,
            "dd" => parsed.disable_download = true,
            "all4" => parsed.test_all = true,
            "sp" => parsed.show_port = true,
            // 带值的字符串参数
            // hu 可带值或不带值
            "hu" => {
                parsed.httping_urls_flag = true;
                parsed.httping = true;
                Self::set_string(value, &mut parsed.httping_urls);
            },
            // 数值型参数
            "t" => Self::set_u16(value, &mut parsed.ping_times),
            "dn" => Self::set_u16(value, &mut parsed.test_count),
            "tp" => Self::set_u16(value, &mut parsed.tcp_port),
            // 浮点型参数
            "tlr" => Self::set_f32(value, &mut parsed.max_loss_rate),
            "sl" => Self::set_f32(value, &mut parsed.min_speed),
            // 其他参数
            "p" => Self::set_u16(value, &mut parsed.print_num),
            "n" => Self::set_usize_clamped(value, &mut parsed.max_threads, 1, 1024), // 带范围限制
            "tn" => Self::set_u32_option(value, &mut parsed.target_num),
            // 时间参数（秒）
            "dt" => Self::set_duration_secs(value, &mut parsed.timeout_duration),
            "timeout" => Self::set_duration_secs(value, &mut parsed.global_timeout_duration),
            // 时间参数（毫秒）
            "tl" => Self::set_duration_ms(value, &mut parsed.max_delay),
            "tll" => Self::set_duration_ms(value, &mut parsed.min_delay),
            // 字符串参数
            "url" => Self::set_string(value, &mut parsed.url),
            "urlist" => Self::set_string(value, &mut parsed.urlist),
            "colo" => Self::set_string(value, &mut parsed.httping_cf_colo),
            "f" => Self::set_string(value, &mut parsed.ip_file),
            "ip" => Self::set_string(value, &mut parsed.ip_text),
            "ipurl" => Self::set_string(value, &mut parsed.ip_url),
            "o" => Self::set_string(value, &mut parsed.output),
            // 未知参数处理
            _ => {
                print_help();  // 显示帮助信息
                eprintln!("错误: 不支持的参数: {}", name);
                std::process::exit(1);  // 退出程序
            }
        }
    }

    /// 设置u16类型参数值
    #[inline(never)] 
    fn set_u16(value: Option<&String>, target: &mut u16) {
        if let Some(v) = value.and_then(|s| s.parse().ok()) {
            *target = v;
        }
    }

    /// 设置可选u32类型参数值
    #[inline(never)] 
    fn set_u32_option(value: Option<&String>, target: &mut Option<u32>) {
        if let Some(v) = value.and_then(|s| s.parse().ok()) {
            *target = Some(v);
        }
    }

    /// 设置f32类型参数值
    #[inline(never)] 
    fn set_f32(value: Option<&String>, target: &mut f32) {
        if let Some(v) = value.and_then(|s| s.parse().ok()) {
            *target = v;
        }
    }

    /// 设置带范围限制的usize类型参数值
    #[inline(never)] 
    fn set_usize_clamped(value: Option<&String>, target: &mut usize, min: usize, max: usize) {
        if let Some(v) = value.and_then(|s| s.parse::<usize>().ok()) {
            *target = v.clamp(min, max);
        }
    }

    /// 设置秒为单位的Duration参数
    #[inline(never)] 
    fn set_duration_secs(value: Option<&String>, target: &mut Option<Duration>) {
        if let Some(v) = value.and_then(|s| s.parse::<u64>().ok()) {
            *target = Some(Duration::from_secs(v));
        }
    }

    /// 设置毫秒为单位的Duration参数
    #[inline(never)] 
    fn set_duration_ms(value: Option<&String>, target: &mut Duration) {
        if let Some(v) = value.and_then(|s| s.parse::<u64>().ok()) {
            *target = Duration::from_millis(v);
        }
    }

    /// 设置字符串类型参数
    #[inline(never)] 
    fn set_string(value: Option<&String>, target: &mut String) {
        if let Some(v) = value {
            *target = v.clone();
        }
    }
}

/// 公开接口：解析并验证参数
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
