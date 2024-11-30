mod types;
mod download;
mod httping;
mod ip;
mod tcping;
mod progress;
mod csv;
mod version;
mod threadpool;

use anyhow::Result;
use std::time::Duration;
use crate::types::{Config, DelayFilter, parse_test_amount};
use crate::csv::PrintResult;
use crate::httping::HttpPing;
use tracing::debug;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const NAME: &str = "CloudflareST-Rust";
const HELP_TEXT: &str = r#"
CloudflareST-Rust

参数：
    -t 4
        延迟测速次数；单个 IP 延迟测速的次数；(默认 4 次)
    -dn 10
        下载测速数量；延迟测速并排序后，从最低延迟起下载测速的数量；(默认 10 个)
    -dt 10
        下载测速时间；单个 IP 下载测速最长时间，不能太短；(默认 10 秒)
    -tp 443
        指定测速端口；延迟测速/下载测速时使用的端口；(默认 443 端口)
    -url https://cf.xiu2.xyz/url
        指定测速地址；延迟测速(HTTPing)/下载测速时使用的地址，默认地址不保证可用性，建议自建；

    -httping
        切换测速模式；延迟测速模式改为 HTTP 协议，所用测试地址为 [-url] 参数；(默认 TCPing)
    -httping-code 200
        有效状态代码；HTTPing 延迟测速时网页返回的有效 HTTP 状态码，仅限一个；(默认 200 301 302)
    -cfcolo HKG,KHH,NRT,LAX,SEA,SJC,FRA,MAD
        匹配指定地区；地区名为当地机场三字码，英文逗号分隔，仅 HTTPing 模式可用；(默认 所有地区)

    -tl 200
        平均延迟上限；只输出低于指定平均延迟的 IP，各上下限条件可搭配使用；(默认 9999 ms)
    -tll 40
        平均延迟下限；只输出高于指定平均延迟的 IP；(默认 0 ms)
    -tlr 0.2
        丢包几率上限；只输出低于/等于指定丢包率的 IP，范围 0.00~1.00，0 过滤掉任何丢包的 IP；(默认 1.00)
    -sl 5
        下载速度下限；只输出高于指定下载速度的 IP，凑够指定数量 [-dn] 才会停止测速；(默认 0.00 MB/s)

    -p 10
        显示结果数量；测速后直接显示指定数量的结果，为 0 时不显示结果直接退出；(默认 10 个)
    -f ip.txt
        IP段数据文件；如路径含有空格请加上引号；支持其他 CDN IP段；(默认 ip.txt)
    -ip 1.1.1.1,2.2.2.2/24,2606:4700::/32
        指定IP段数据；直接通过参数指定要测速的 IP 段数据，英文逗号分隔；(默认 空)
    -o result.csv
        写入结果文件；如路径含有空格请加上引号；值为空时不写入文件 [-o ""]；(默认 result.csv)

    -dd
        禁用下载测速；禁用后测速结果会按延迟排序 (默认按下载速度排序)；(默认 启用)
    -all4
        测速全部的 IPv4；(IPv4 默认每 64 个随机测速 1 个 IP)
    -more6
        测试更多 IPv6；(表示 -v6 18，即每个 CIDR 测速 2^18 即 262144 个)
    -lots6
        测试较多 IPv6；(表示 -v6 16，即每个 CIDR 测速 2^16 即 65536 个)
    -many6
        测试很多 IPv6；(表示 -v6 12，即每个 CIDR 测速 2^12 即 4096 个)
    -some6
        测试一些 IPv6；(表示 -v6 8，即每个 CIDR 测 2^8 即 256 个)
    -many4
        测试一点 IPv4；(表示 -v4 12，即每个 CIDR 测速 2^12 即 4096 个)

    -v4
        指定 IPv4 测试数量 (2^n±m，例如 -v4 0+12 表示 2^0+12 即每个 CIDR 测速 13 个)
    -v6
        指定 IPv6 测试数量 (2^n±m，例如 -v6 18-6 表示 2^18-6 即每个 CIDR 测速 262138 个)

    -v
        打印程序版本 + 检查版本更新
    -h
        打印帮助说明
"#;

// 新增参数解析结构体
struct Args {
    args: Vec<(String, Option<String>)>,
}

impl Args {
    fn new() -> Self {
        Self { args: Vec::new() }
    }

    fn parse(args: Vec<String>) -> Self {
        let mut parsed = Self::new();
        let mut i = 1;  // 跳过程序名
        
        while i < args.len() {
            let arg = &args[i];
            
            // 确保是参数标志
            if !arg.starts_with('-') {
                i += 1;
                continue;
            }

            let name = arg.trim_start_matches('-').to_string();
            
            // 检查是否是无值标志参数
            match name.as_str() {
                "v" | "h" | "httping" | "dd" | "all4" | "more6" | "lots6" | "many6" | "some6" | "many4" => {
                    parsed.args.push((name, None));
                    i += 1;
                    continue;
                }
                _ => {}
            }
            
            // 处理带值的参数
            if i + 1 < args.len() && !args[i + 1].starts_with('-') {
                parsed.args.push((name, Some(args[i + 1].clone())));
                i += 2;  // 跳过参数名和值
            } else {
                i += 1;
            }
        }

        parsed
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.args.iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| v.as_deref())
    }

    fn has(&self, name: &str) -> bool {
        self.args.iter().any(|(n, _)| n == name)
    }
}

fn init_tracing() {
    if cfg!(feature = "debug") {
        use tracing_subscriber::{fmt, EnvFilter};
        
        fmt()
            .with_env_filter(EnvFilter::from_default_env()
                .add_directive(tracing::Level::DEBUG.into()))
            .with_target(false)
            .with_thread_ids(true)
            .with_thread_names(true)
            .with_file(true)
            .with_line_number(true)
            .init();
    }
}

fn main() -> Result<()> {
    init_tracing();  // 初始化日志
    
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            debug!("启动异步运行时");
            let args = Args::parse(std::env::args().collect());

            // 处理无值参数
            if args.has("h") {
                println!("{}", HELP_TEXT);
                wait_for_input();
                return Ok(());
            }

            if args.has("v") {
                print_version();
                println!("检查版本更新中...");
                if let Some(new_version) = version::check_update().await {
                    println!("*** 发现新版本 [{}]！请前往 [https://github.com/GuangYu-yu/CloudflareST-Rust] 更新！ ***", new_version);
                } else {
                    println!("当前为最新版本 [{}]！", VERSION);
                }
                wait_for_input();
                return Ok(());
            }

            // 创建配置
            let mut config = Config::default();

            // 解析参数
            if let Some(v) = args.get("t") {
                config.ping_times = v.parse().unwrap_or(4);
            }
            if let Some(v) = args.get("dn") {
                config.test_count = v.parse().unwrap_or(10);
            }
            if let Some(v) = args.get("dt") {
                config.download_time = Duration::from_secs(v.parse().unwrap_or(10));
            }
            if let Some(v) = args.get("tp") {
                config.tcp_port = v.parse().unwrap_or(443);
            }
            if let Some(v) = args.get("url") {
                config.url = v.to_string();
            }
            if args.has("httping") {
                config.httping = true;
            }
            if let Some(v) = args.get("httping-code") {
                config.httping_status_code = v.parse().unwrap_or(200);
            }
            if let Some(v) = args.get("cfcolo") {
                config.httping_cf_colo = v.to_string();
            }
            if let Some(v) = args.get("tl") {
                config.max_delay = Duration::from_millis(v.parse().unwrap_or(9999));
            }
            if let Some(v) = args.get("tll") {
                config.min_delay = Duration::from_millis(v.parse().unwrap_or(0));
            }
            if let Some(v) = args.get("tlr") {
                config.max_loss_rate = v.parse().unwrap_or(1.0);
            }
            if let Some(v) = args.get("sl") {
                config.min_speed = v.parse().unwrap_or(0.0);
            }
            if let Some(v) = args.get("p") {
                config.print_num = v.parse().unwrap_or(10);
            }
            if let Some(v) = args.get("f") {
                config.ip_file = v.to_string();
            }
            if let Some(v) = args.get("ip") {
                config.ip_text = v.to_string();
            }
            if let Some(v) = args.get("o") {
                config.output = v.to_string();
            }
            if args.has("dd") {
                config.disable_download = true;
            }
            if args.has("all4") {
                config.test_all = true;
            }
            if args.has("more6") {
                config.ipv6_amount = Some(262144);
            }
            if args.has("lots6") {
                config.ipv6_amount = Some(65536);
            }
            if args.has("many6") {
                config.ipv6_amount = Some(4096);
            }
            if args.has("some6") {
                config.ipv6_amount = Some(256);
            }
            if args.has("many4") {
                config.ipv4_amount = Some(4096);
            }
            if args.has("many4") {
                config.ipv4_num_mode = Some("many".to_string());
            }
            if args.has("more6") {
                config.ipv6_num_mode = Some("more".to_string());
            }
            if args.has("lots6") {
                config.ipv6_num_mode = Some("lots".to_string());
            }
            if args.has("many6") {
                config.ipv6_num_mode = Some("many".to_string());
            }
            if args.has("some6") {
                config.ipv6_num_mode = Some("some".to_string());
            }
            if let Some(v) = args.get("v4") {
                config.ipv4_amount = Some(parse_test_amount(v, true));
            }
            if let Some(v) = args.get("v6") {
                config.ipv6_amount = Some(parse_test_amount(v, false));
            }

            println!("CloudflareST-Rust {}\n", VERSION);
            
            ip::init_rand_seed();
            check_config(&config);

            // 执行测速
            let ping_data = if config.httping {
                // 使用 HTTP 测速
                let http_ping = HttpPing::new(config.clone(), Some(&config.httping_cf_colo));
                let ips = ip::load_ip_ranges_concurrent(&config).await?;
                http_ping.http_ping_all(&config, &ips).await
            } else {
                // 使用 TCP 测速
                let ping = tcping::new_ping(config.clone()).await?;
                ping.run().await?
            };

            let ping_data = ping_data.filter_delay().filter_loss_rate();

            let mut speed_data = download::test_download_speed(&mut config, ping_data).await?;
            csv::export_csv(&mut speed_data, &config).await?;
            speed_data.print();

            wait_for_input();
            Ok(())
        })
}

fn check_config(config: &Config) {
    if config.min_speed > 0.0 && config.max_delay == Duration::from_millis(9999) {
        println!("[提示] 使用下载速度下限时，建议同时设置延迟上限，避免测速时间过长");
    }
}

fn print_version() {
    println!("{} v{}", NAME, VERSION);
    
    #[cfg(debug_assertions)]
    println!("Debug Build");
    
    #[cfg(not(debug_assertions))]
    println!("Release Build");
}

fn wait_for_input() {
    println!("\n按回车键退出...");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap_or_default();
}
