use std::env;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use crate::{error_and_exit, warning_println};
use crate::interface::{InterfaceParamResult, process_interface_param};

// 非TLS端口数组
const NON_TLS_PORTS: [u16; 7] = [80, 8080, 8880, 2052, 2082, 2086, 2095];
// TLS端口数组
const TLS_PORTS: [u16; 6] = [443, 2053, 2083, 2087, 2096, 8443];

/// 命令行参数配置结构体
#[derive(Clone)]
pub(crate) struct Args {
    // 网络测试参数
    #[cfg(feature = "icmp")]
    pub(crate) icmp_ping: bool,                    // 是否使用ICMP Ping测速
    pub(crate) ping_times: u16,                    // Ping测试次数
    pub(crate) tcp_port: u16,                      // 端口号
    pub(crate) url: String,                        // 测速URL
    pub(crate) httping: String,                    // HTTPing
    pub(crate) httping_code: String,               // HTTPing要求的HTTP状态码
    pub(crate) httping_cf_colo: String,            // 指定数据中心
    pub(crate) max_delay: Duration,                // 最大可接受延迟
    pub(crate) min_delay: Duration,                // 最小可接受延迟
    pub(crate) max_loss_rate: f32,                 // 最大丢包率阈值
    pub(crate) test_count: usize,                  // 所需达到下载速度下限的IP数量
    pub(crate) timeout_duration: Option<Duration>, // 单次下载测速的持续时间
    pub(crate) min_speed: f32,                     // 最低下载速度要求(MB/s)
    pub(crate) disable_download: bool,             // 是否禁用下载测试

    // 结果处理参数
    pub(crate) target_num: Option<usize>, // Ping所需可用IP数量
    pub(crate) print_num: u16,            // 显示结果数量
    pub(crate) ip_file: String,           // IP列表文件路径
    pub(crate) ip_text: String,           // 直接指定的IP
    pub(crate) output: Option<String>,    // 结果输出文件

    // 功能开关
    pub(crate) test_all_ipv4: bool,  // 测试所有IPv4
    pub(crate) help: bool,           // 打印帮助信息
    pub(crate) show_port: bool,      // 在结果中显示端口

    // 高级设置
    pub(crate) global_timeout_duration: Option<Duration>, // 全局超时设置
    pub(crate) max_threads: usize,                        // 最大线程数
    pub(crate) interface_config: Arc<InterfaceParamResult>,  // 接口配置
}

impl Args {
    /// 创建默认参数配置
    pub(crate) fn new() -> Self {
        Self {
            #[cfg(feature = "icmp")]
            icmp_ping: false,
            ping_times: 4,
            tcp_port: 443,
            url: String::new(),
            httping: String::new(),
            httping_code: String::new(),
            httping_cf_colo: String::new(),
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
            output: Some("result.csv".to_string()),
            test_all_ipv4: false,
            help: false,
            show_port: false,
            global_timeout_duration: None,
            max_threads: 256,
            interface_config: Arc::new(InterfaceParamResult::default()),
        }
    }

    // 字符串转换为数字
    fn parse_or<T>(value_opt: Option<String>, default: T) -> T
    where
        T: std::str::FromStr + Copy,
    {
        value_opt.map_or(default, |s| s.parse().unwrap_or(default))
    }

    // 字符串赋值
    fn assign_string(target: &mut String, value_opt: Option<String>) {
        if let Some(v) = value_opt {
            *target = v;
        }
    }

    /// 解析命令行参数
    pub(crate) fn parse() -> Self {
        let args: Vec<String> = env::args().collect();
        let mut parsed = Self::new();
        let vec = Self::parse_args_to_vec(&args);

        // 标记是否使用了 -tp 参数
        let mut use_tp = false;

        for (k, v_opt) in vec {
            match k.as_str() {
                // 布尔参数
                "h" | "help" => parsed.help = true,
                "httping" => Self::assign_string(&mut parsed.httping, v_opt),
                "dd" => parsed.disable_download = true,
                "all4" => parsed.test_all_ipv4 = true,
                "sp" => parsed.show_port = true,
                #[cfg(feature = "icmp")]
                "ping" => parsed.icmp_ping = true,

                // 数值参数
                "t" => {
                    parsed.ping_times = Self::parse_or(v_opt, parsed.ping_times).clamp(1, u16::MAX);
                }
                "dn" => {
                    parsed.test_count = Self::parse_or(v_opt, parsed.test_count).clamp(1, usize::MAX);
                }
                "tp" => {
                    use_tp = true;
                    parsed.tcp_port = Self::parse_or(v_opt, parsed.tcp_port).clamp(1, u16::MAX);
                }
                "p" => {
                    parsed.print_num = Self::parse_or(v_opt, parsed.print_num).clamp(1, u16::MAX);
                }
                "tlr" => {
                    parsed.max_loss_rate = Self::parse_or(v_opt, parsed.max_loss_rate).clamp(0.0, 1.0);
                }
                "sl" => {
                    parsed.min_speed = Self::parse_or(v_opt, parsed.min_speed).clamp(0.0, f32::MAX);
                }
                "tn" => parsed.target_num = v_opt.and_then(|s| s.parse().ok()),
                "n" => {
                    parsed.max_threads = Self::parse_or(v_opt, parsed.max_threads).clamp(1, 1024);
                }
                // 时间参数
                "dt" => {
                    let seconds = Self::parse_or(v_opt, parsed.timeout_duration.map(|d| d.as_secs()).unwrap());
                    parsed.timeout_duration = Some(Duration::from_secs(seconds.clamp(1, 120)));
                }
                "timeout" => {
                    parsed.global_timeout_duration = v_opt
                        .and_then(|v| v.parse::<u64>().ok())
                        .map(|s| Duration::from_secs(s.clamp(1, 36000)));
                }
                "tl" => {
                    let ms = Self::parse_or::<u64>(v_opt, parsed.max_delay.as_millis().try_into().unwrap());
                    parsed.max_delay = Duration::from_millis(ms.clamp(0, 2000));
                }
                "tll" => {
                    let max_allowed = parsed.max_delay.as_millis().try_into().unwrap();
                    parsed.min_delay = Duration::from_millis(Self::parse_or::<u64>(v_opt, parsed.min_delay.as_millis().try_into().unwrap()).clamp(0, max_allowed));
                }
                // 字符串参数
                "url" => Self::assign_string(&mut parsed.url, v_opt),
                "hc" => Self::assign_string(&mut parsed.httping_code, v_opt),
                "colo" => Self::assign_string(&mut parsed.httping_cf_colo, v_opt),
                "f" => Self::assign_string(&mut parsed.ip_file, v_opt),
                "ip" => Self::assign_string(&mut parsed.ip_text, v_opt),
                "o" => parsed.output = v_opt,
                "intf" => {
                    if let Some(ref interface) = v_opt {
                        // 调用 interface.rs 中的函数处理接口参数
                        parsed.interface_config = Arc::new(process_interface_param(interface));

                        // 检查参数是否有效（既不是IP也不是有效的接口名）
                        if !parsed.interface_config.is_valid_interface {
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

        // 若启用 httping 且未使用 -tp，则根据HTTPing URL设置默认端口
        if !use_tp && !parsed.httping.is_empty() && parsed.httping.starts_with("http://") {parsed.tcp_port = 80}

        parsed
    }

    // 解析命令行
    fn parse_args_to_vec(args: &[String]) -> Vec<(String, Option<String>)> {
        let mut iter = args.iter().skip(1).peekable();
        let mut result = Vec::new();

        while let Some(arg) = iter.next() {
            if arg.starts_with('-') {
                let key = arg.trim_start_matches('-').to_string();
                let value = iter.peek()
                    .filter(|next| !next.starts_with('-'))
                    .map(|next| next.to_string());
                
                if value.is_some() {
                    iter.next(); // 消耗掉值
                }
                
                result.push((key, value));
            }
        }
        
        result
    }
}

/// 解析并验证参数
pub(crate) fn parse_args() -> Args {
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

    if args.ip_file.is_empty() && args.ip_text.is_empty() {
        error_and_exit(format_args!("必须指定一个或多个 IP 来源参数 (-f 或 -ip)"));
    }

    if !args.disable_download && args.url.is_empty() {
        error_and_exit(format_args!("必须设置 -url 参数，或使用 -dd 参数来禁用下载测速"));
    }

    if args.disable_download && !args.url.is_empty() {
        warning_println(format_args!("使用了 -dd 参数，但仍设置了 -url 参数"));
    }

    // 验证HTTPing URL格式
    if !args.httping.is_empty() && !args.httping.starts_with("http://") && !args.httping.starts_with("https://") {
        error_and_exit(format_args!("HTTPing URL必须以协议前缀开头"));
    }

    // 检查端口与协议的匹配情况
    let is_mismatch = 
        // HTTPing相关检查
        (!args.httping.is_empty() && (
            // 场景1：使用 HTTP 但指定了TLS端口
            (args.httping.starts_with("http://") && TLS_PORTS.contains(&args.tcp_port)) ||
            // 场景2：使用 HTTPS 但指定了非TLS端口
            (args.httping.starts_with("https://") && NON_TLS_PORTS.contains(&args.tcp_port))
        )) ||
        
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
        } else if in_escape {
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

pub(crate) fn print_help() {
    const HELP_ARGS: &[(&str, &str, &str)] = &[
        // 目标参数
        ("", "目标参数", ""), // 标记标题
        ("-f", "从指定文件名或文件路径获取 IP 或 CIDR", "未指定"),
        ("-ip", "直接指定 IP 或 CIDR（多个用逗号分隔）", "未指定"),
        ("-url", "TLS 模式的 Httping 或下载测速所使用的 URL", "未指定"),
        ("-tp", "测速端口", "80 / 443"),
        
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
        ("-httping", "使用 HTTPing 测速，并指定其地址", "否"),
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
    
    // 构建完整的帮助信息
    let mut help_text = String::new();
    
    for (name, desc, default) in HELP_ARGS.iter() {
        if name.is_empty() {
            // 标题行
            help_text.push('\n');
            // 添加加粗洋红的标题
            help_text.push_str(&format!("\x1b[1;35m{}\x1b[0m\n", desc));
        } else {
            // 1. 格式化参数名：绿色 (\x1b[32m)
            let name_colored = format!("\x1b[32m{}\x1b[0m", name);
            let name_display_width = approximate_display_width_no_color(&name_colored);
            let name_padding = " ".repeat(11usize.saturating_sub(name_display_width));
            
            // 2. 格式化描述 (默认颜色)
            let desc_display_width = approximate_display_width_no_color(desc);
            let desc_padding = " ".repeat(45usize.saturating_sub(desc_display_width));

            // 3. 格式化默认值：暗淡色 (\x1b[2m)
            let default_colored = format!("\x1b[2m{}\x1b[0m", default);
            let default_display_width = approximate_display_width_no_color(&default_colored);
            let default_padding = " ".repeat(15usize.saturating_sub(default_display_width));

            // 4. 构建完整的参数行并添加到帮助文本
            help_text.push_str(&format!(
                " {}{}{}{}{}{}\n",
                name_colored,
                name_padding,
                desc,
                desc_padding,
                default_colored,
                default_padding
            ));
        }
    }
    
    print!("{}", help_text);
}