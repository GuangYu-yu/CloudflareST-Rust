use std::env;
use std::time::Duration;
use colored::*;  // 用于终端彩色输出
use prettytable::{Table, Row, Cell, format};

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
    pub httping_code: String,             // HTTPing使用的HTTP状态码
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
            httping_code: String::new(),
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
                "hc" => parsed.httping_code = v_opt.unwrap_or_else(|| parsed.httping_code.clone()),
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

/// 打印帮助信息
pub fn print_help() {
    // 打印标题
    println!("{}", "# CloudflareST-Rust".bold().blue());

    // 创建表格
    let mut table = Table::new();

    // 设置表格样式（可选）
    table.set_format(*format::consts::FORMAT_CLEAN);

    // Helper：插入参数行
    macro_rules! add_arg {
        ($name:expr, $desc:expr, $default:expr) => {
            table.add_row(Row::new(vec![
                Cell::new(&format!(" {:<12}", $name.green())),   // 参数列：缩进+左对齐+宽度
                Cell::new(&format!("{:<16}", $desc)),     // 描述列
                Cell::new(&format!("{:<10}", $default.dimmed())),  // 默认值列
            ]));
        };
    }

    add_arg!("-url", "TLS 模式的 Httping 或下载测速所使用的 URL", "未指定");
    add_arg!("-urlist", "从 URL 内读取测速地址列表", "未指定");
    add_arg!("-f", "从指定文件名或文件路径获取 IP 或 CIDR", "未指定");
    add_arg!("-ip", "直接指定 IP 或 CIDR（多个用逗号分隔）", "未指定");
    add_arg!("-ipurl", "从 URL 读取 IP 或 CIDR", "未指定");
    add_arg!("-timeout", "程序超时退出时间（秒）", "不限制");

    add_arg!("-t", "延迟测速次数", "4");
    add_arg!("-dn", "下载测速所需符合要求的结果数量", "10");
    add_arg!("-dt", "下载测速时间（秒）", "10");
    add_arg!("-tp", "测速端口", "443");
    add_arg!("-all4", "测速全部 IPv4 地址", "否");
    add_arg!("-tn", "当 Ping 到指定可用数量，提前结束 Ping", "否");

    add_arg!("-httping", "使用非 TLS 模式的 Httping", "否");
    add_arg!("-dd", "禁用下载测速", "否");
    add_arg!("-hc", "指定 HTTPing 的状态码（例如：200,301,302）", "未指定");
    add_arg!("-hu", "使用 HTTPS 进行延迟测速，可指定测速地址", "否");
    add_arg!("-colo", "指定地区（例如：HKG,SJC）", "未指定");
    add_arg!("-n", "延迟测速的线程数量", "256");

    add_arg!("-tl", "延迟上限（毫秒）", "2000");
    add_arg!("-tll", "延迟下限（毫秒）", "0");
    add_arg!("-tlr", "丢包率上限", "1.00");
    add_arg!("-sl", "下载速度下限（MB/s）", "0.00");
    add_arg!("-p", "终端显示结果数量", "10");
    add_arg!("-sp", "结果中带端口号", "否");
    add_arg!("-o", "输出结果文件（文件名或文件路径）", "result.csv");

    table.printstd();
}
