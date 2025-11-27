use std::env;
use std::path::Path;
use std::time::Duration;
use crate::{error_println, warning_println};
use crate::interface::{InterfaceIps, process_interface_param};

// 非TLS端口数组
const NON_TLS_PORTS: [u16; 7] = [80, 8080, 8880, 2052, 2082, 2086, 2095];
// TLS端口数组
const TLS_PORTS: [u16; 6] = [443, 2053, 2083, 2087, 2096, 8443];

/// 命令行参数配置结构体
#[derive(Clone)]
pub struct Args {
    // 网络测试参数
    #[cfg(feature = "icmp")]
    pub icmp_ping: bool,                  // 是否使用ICMP Ping测速
    pub ping_times: u16,                    // Ping测试次数
    pub tcp_port: u16,                      // TCP端口号
    pub url: String,                        // 单个测速URL
    pub urlist: String,                     // URL列表文件路径
    pub httping: bool,                      // 是否启用HTTPing测试
    pub httping_code: String,               // HTTPing使用的HTTP状态码
    pub httping_cf_colo: String,            // 指定Cloudflare地区代码
    pub httping_urls: Option<String>,       // HTTPing使用的URL列表
    pub max_delay: Duration,                // 最大可接受延迟
    pub min_delay: Duration,                // 最小可接受延迟
    pub max_loss_rate: f32,                 // 最大丢包率阈值
    pub test_count: u16,                    // 下载测试次数
    pub timeout_duration: Option<Duration>, // 单次测试超时时间
    pub min_speed: f32,                     // 最低下载速度要求(MB/s)
    pub disable_download: bool,             // 是否禁用下载测试

    // 结果处理参数
    pub target_num: Option<u32>, // 提前结束测试的目标数量
    pub print_num: u16,          // 显示结果数量
    pub ip_file: String,         // IP列表文件路径
    pub ip_text: String,         // 直接指定的IP列表
    pub ip_url: String,          // 获取IP的URL地址
    pub output: Option<String>,  // 结果输出文件

    // 功能开关
    pub test_all: bool,  // 是否测试所有IP
    pub help: bool,      // 是否显示帮助信息
    pub show_port: bool, // 是否在结果中显示端口

    // 高级设置
    pub global_timeout_duration: Option<Duration>, // 全局超时设置
    pub max_threads: usize,                        // 最大线程数
    pub interface: Option<String>,                 // 网络接口名或 IP 地址
    pub interface_ips: Option<InterfaceIps>, // 接口的 IPv4 和 IPv6 地址
}

// 错误处理
pub fn error_and_exit(args: std::fmt::Arguments<'_>) -> ! {
    error_println(args);
    std::process::exit(1);
}

impl Args {
    /// 创建默认参数配置
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "icmp")]
            icmp_ping: false,
            ping_times: 4, // 默认Ping测试4次
            tcp_port: 443, // 默认使用443端口
            url: String::new(),
            urlist: String::new(),
            httping: false,
            httping_code: String::new(),
            httping_cf_colo: String::new(),
            httping_urls: None,
            max_delay: Duration::from_millis(2000), // 默认最大延迟2000ms
            min_delay: Duration::from_millis(0),    // 默认最小延迟0ms
            max_loss_rate: 1.0,                     // 默认最大丢包率100%
            test_count: 10,                         // 默认下载测试10次
            timeout_duration: Some(Duration::from_secs(10)), // 默认单次超时10秒
            min_speed: 0.0,                         // 默认最低速度0MB/s
            disable_download: false,
            target_num: None,
            print_num: 10, // 默认显示前10个结果
            ip_file: String::new(),
            ip_text: String::new(),
            ip_url: String::new(),
            output: Some("result.csv".to_string()), // 默认输出文件
            test_all: false,
            help: false,
            show_port: false,
            global_timeout_duration: None, // 默认无全局超时
            max_threads: 256,              // 默认最大线程数256
            interface: None,
            interface_ips: None,
        }
    }

    // 私有解析助手函数
    fn parse_u16(value_opt: Option<String>, default: u16) -> u16 {
        value_opt
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(default)
    }

    fn parse_f32(value_opt: Option<String>, default: f32) -> f32 {
        value_opt
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(default)
    }

    fn parse_u64(value_opt: Option<String>, default: u64) -> u64 {
        value_opt
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(default)
    }

    fn parse_usize(value_opt: Option<String>, default: usize) -> usize {
        value_opt
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(default)
    }

    /// 解析命令行参数
    pub fn parse() -> Self {
        let args: Vec<String> = env::args().collect();
        let mut parsed = Self::new();
        let vec = Self::parse_args_to_vec(&args);

        // 标记是否使用了 -tp 参数
        let mut use_tp = false;

        for (k, v_opt) in vec {
            match k.as_str() {
                // 布尔参数
                "h" | "help" => parsed.help = true,
                "httping" => parsed.httping = true,
                "dd" => parsed.disable_download = true,
                "all4" => parsed.test_all = true,
                "sp" => parsed.show_port = true,
                #[cfg(feature = "icmp")]
                "ping" => parsed.icmp_ping = true,

                // hu 可以有值也可以没有值
                "hu" => {
                    parsed.httping = true;
                    parsed.httping_urls = Some(v_opt.unwrap_or_default());
                }

                // 数值参数
                "t" => {
                    parsed.ping_times = Self::parse_u16(v_opt, parsed.ping_times).clamp(1, u16::MAX);
                }
                "dn" => {
                    parsed.test_count = Self::parse_u16(v_opt, parsed.test_count).clamp(1, u16::MAX);
                }
                "tp" => {
                    use_tp = true;
                    parsed.tcp_port = Self::parse_u16(v_opt, parsed.tcp_port).clamp(1, u16::MAX);
                }
                "p" => {
                    parsed.print_num = Self::parse_u16(v_opt, parsed.print_num).clamp(1, u16::MAX);
                }
                "tlr" => {
                    parsed.max_loss_rate = Self::parse_f32(v_opt, parsed.max_loss_rate).clamp(0.0, 1.0);
                }
                "sl" => {
                    parsed.min_speed = Self::parse_f32(v_opt, parsed.min_speed).clamp(0.0, f32::MAX);
                }
                "tn" => parsed.target_num = v_opt.and_then(|s| s.parse().ok()),
                "n" => {
                    parsed.max_threads = Self::parse_usize(v_opt, parsed.max_threads).clamp(1, 1024);
                }
                // 时间参数
                "dt" => {
                    let seconds = Self::parse_u64(v_opt, parsed.timeout_duration.map(|d| d.as_secs()).unwrap());
                    parsed.timeout_duration = Some(Duration::from_secs(seconds.clamp(1, 120)));
                }
                "timeout" => {
                    parsed.global_timeout_duration = v_opt
                        .and_then(|v| v.parse::<u64>().ok())
                        .map(|s| Duration::from_secs(s.clamp(1, 36000)));
                }
                "tl" => {
                    let ms = Self::parse_u64(v_opt, parsed.max_delay.as_millis() as u64);
                    parsed.max_delay = Duration::from_millis(ms.clamp(0, 2000));
                }
                "tll" => {
                    let ms = Self::parse_u64(v_opt, parsed.min_delay.as_millis() as u64);
                    parsed.min_delay = Duration::from_millis(ms.clamp(0, parsed.max_delay.as_millis() as u64));
                }
                // 字符串参数
                "url" => {
                    if let Some(v) = v_opt {
                        parsed.url = v;
                    }
                }
                "urlist" => {
                    if let Some(v) = v_opt {
                        parsed.urlist = v;
                    }
                }
                "hc" => {
                    if let Some(v) = v_opt {
                        parsed.httping_code = v;
                    }
                }
                "colo" => {
                    if let Some(v) = v_opt {
                        parsed.httping_cf_colo = v;
                    }
                }
                "f" => {
                    if let Some(v) = v_opt {
                        parsed.ip_file = v;
                    }
                }
                "ip" => {
                    if let Some(v) = v_opt {
                        parsed.ip_text = v;
                    }
                }
                "ipurl" => {
                    if let Some(v) = v_opt {
                        parsed.ip_url = v;
                    }
                }
                "o" => parsed.output = v_opt,
                "intf" => {
                    parsed.interface = v_opt;

                    if let Some(ref interface) = parsed.interface {
                        // 调用 interface.rs 中的函数处理接口参数
                        let result = process_interface_param(interface);

                        parsed.interface_ips = result.interface_ips;

                        // 检查参数是否有效（既不是IP也不是有效的接口名）
                        if !result.is_valid_interface {
                            error_and_exit(format_args!("无效的绑定: {}", interface));
                        }
                    }
                }

                // 无效参数：打印错误并退出
                _ => {
                    print_help();
                    error_and_exit(format_args!("无效的参数: {k}"));
                }
            }
        }

        // 若启用 httping 且未使用 -hu 或 -tp，则默认端口为 80
        if parsed.httping && parsed.httping_urls.is_none() && !use_tp {
        parsed.tcp_port = 80;
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
                        Some(iter.next().unwrap().to_string())
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

    if args.help {
        print_help();
        std::process::exit(0);
    }

    if !args.ip_file.is_empty() && !Path::new(&args.ip_file).exists() {
        error_and_exit(format_args!("指定的文件不存在"));
    }

    // 检查输出文件是否被占用（仅Windows）
    #[cfg(target_os = "windows")]
    if let Some(ref output_file) = args.output {
        let output_path = Path::new(output_file);
        if output_path.exists() {
            std::fs::OpenOptions::new().write(true).open(output_path).unwrap_or_else(|e| {
                let msg = match e.raw_os_error() {
                    Some(32) => format!("输出文件 '{}' 正被其他程序占用", output_path.display()),
                    _ => format!("无法写入输出文件 '{}': {}", output_path.display(), e),
                };
                error_and_exit(format_args!("{}", msg));
            });
        }
    }

    if args.ip_file.is_empty() && args.ip_url.is_empty() && args.ip_text.is_empty() {
        error_and_exit(format_args!("必须指定一个或多个 IP 来源参数 (-f, -ipurl 或 -ip)"));
    }

    // 先检查 -hu 参数的特殊情况
    if args.httping_urls.is_some()
        && args.httping_urls.as_ref().unwrap().is_empty()
        && args.url.is_empty()
        && args.urlist.is_empty()
    {
        error_and_exit(format_args!("使用 -hu 参数并且没有传入测速地址时，必须通过 -url 或 -urlist 参数指定测速地址"));
    }
    // 然后检查一般的下载测试情况，但排除已经被 -hu 检查过的情况
    else if !args.disable_download && args.url.is_empty() && args.urlist.is_empty() {
        error_and_exit(format_args!("未设置测速地址，在没有使用 -dd 参数时，请使用 -url 或 -urlist 参数指定下载测速的测速地址"));
    }

    if args.disable_download
        && (!args.url.is_empty() || !args.urlist.is_empty())
        && !(args.httping_urls.is_some() && args.httping_urls.as_ref().unwrap().is_empty())
    {
        warning_println(format_args!("使用了 -dd 参数，但仍设置了 -url 或 -urlist，且未用于 -hu"));
    }

    // 检查端口与协议的匹配情况
    let is_mismatch = 
        // 场景1：使用-httping参数但指定了TLS端口
        (args.httping && TLS_PORTS.contains(&args.tcp_port)) ||
        
        // 场景2：使用-hu参数但指定了非TLS端口
        (args.httping_urls.is_some() && NON_TLS_PORTS.contains(&args.tcp_port)) ||
        
        // 场景3：下载测试中URL协议与端口不匹配
        (!args.disable_download && !args.url.is_empty() && {
            let url_lower = args.url.to_lowercase();
            // HTTP URL配TLS端口，或HTTPS URL配非TLS端口
            (url_lower.starts_with("http:") && TLS_PORTS.contains(&args.tcp_port)) ||
            (url_lower.starts_with("https:") && NON_TLS_PORTS.contains(&args.tcp_port))
        });

    if is_mismatch {
        warning_println(format_args!("端口与协议可能不匹配！"));
    }

    args
}

// 计算显示宽度
fn approximate_display_width_no_color(s: &str) -> usize {
    let mut width = 0;
    let mut in_escape = false; 

    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
            continue;
        }
        if in_escape {
            if c == 'm' || c.is_alphabetic() {
                in_escape = false;
            }
            continue;
        }
        // 非 ASCII (中文) 宽度为 2，ASCII 宽度为 1
        width += if c.is_ascii() { 1 } else { 2 };
    }
    width
}

// 格式化和打印单个参数行
fn print_arg_row(name: &str, desc: &str, default: &str) {
    // 固定的列宽
    const COL_NAME_WIDTH: usize = 11;
    const COL_DESC_WIDTH: usize = 45;
    const COL_DEFAULT_WIDTH: usize = 15;

    // 1. 格式化参数名：绿色 (\x1b[32m)
    let name_colored = format!("\x1b[32m{}\x1b[0m", name);
    let name_display_width = approximate_display_width_no_color(&name_colored);
    let name_padding = COL_NAME_WIDTH.saturating_sub(name_display_width);
    
    // 2. 格式化描述 (默认颜色)
    let desc_display_width = approximate_display_width_no_color(desc);
    let desc_padding = COL_DESC_WIDTH.saturating_sub(desc_display_width);

    // 3. 格式化默认值：暗淡色 (\x1b[2m)
    let default_colored = format!("\x1b[2m{}\x1b[0m", default);
    let default_display_width = approximate_display_width_no_color(&default_colored);
    let default_padding = COL_DEFAULT_WIDTH.saturating_sub(default_display_width);

    // 4. 打印整行 (左侧增加 1 个空格作为缩进)
    println!(
        " {}{}{}{}{}{}",
        name_colored,
        " ".repeat(name_padding),
        desc,
        " ".repeat(desc_padding),
        default_colored,
        " ".repeat(default_padding)
    );
}


pub fn print_help() {
    const HELP_ARGS: &[(&str, &str, &str)] = &[
        // 目标参数
        ("", "目标参数", ""), // 标记标题
        ("-f", "从指定文件名或文件路径获取 IP 或 CIDR", "未指定"),
        ("-ip", "直接指定 IP 或 CIDR（多个用逗号分隔）", "未指定"),
        ("-ipurl", "从 URL 读取 IP 或 CIDR", "未指定"),
        ("-url", "TLS 模式的 Httping 或下载测速所使用的 URL", "未指定"),
        ("-urlist", "从 URL 内读取测速地址列表", "未指定"),
        ("-tp", "测速端口", "80 或 443"),
        
        // 测试参数
        ("", "测试参数", ""), // 标记标题
        ("-t", "延迟测速次数", "4"),
        ("-dt", "下载测速时间（秒）", "10"),
        ("-dn", "下载测速所需符合要求的结果数量", "10"),
        ("-n", "延迟测速的线程数量", "256"),
        ("-tn", "当 Ping 到指定可用数量，提前结束 Ping", "否"),
        ("-intf", "绑定到指定接口名或 IP", "未指定"),

        // 控制参数
        ("", "控制参数", ""), // 标记标题
        ("-httping", "使用非 TLS 模式的 Httping", "否"),
        ("-hu", "使用 HTTPS 进行延迟测速，可指定测速地址", "否"),
        #[cfg(feature = "icmp")]
        ("-ping", "使用 ICMP Ping 进行延迟测速", "否"),
        ("-dd", "禁用下载测速", "否"),
        ("-all4", "测速全部 IPv4 地址", "否"),
        ("-timeout", "程序超时退出时间（秒）", "不限制"),

        // 过滤参数
        ("", "过滤参数", ""), // 标记标题
        ("-tl", "延迟上限（毫秒）", "2000"),
        ("-tll", "延迟下限（毫秒）", "0"),
        ("-tlr", "丢包率上限", "1.00"),
        ("-sl", "下载速度下限（MB/s）", "0.00"),
        ("-hc", "指定 HTTPing 的状态码（例如：200,301,302）", "未指定"),
        ("-colo", "指定地区（例如：HKG,SJC）", "未指定"),

        // 结果参数
        ("", "结果参数", ""), // 标记标题
        ("-p", "终端显示结果数量", "10"),
        ("-sp", "结果中带端口号", "否"),
        ("-o", "输出结果文件（文件名或文件路径）", "result.csv"),
    ];
    
    // 打印逻辑
    for (name, desc, default) in HELP_ARGS.iter() {
        if name.is_empty() {
            // 标题行
            println!();
            // 打印加粗洋红的标题
            println!("\x1b[1;35m{}\x1b[0m", desc);
            continue;
        }

        // 打印参数行
        print_arg_row(name, desc, default);
    }
}
