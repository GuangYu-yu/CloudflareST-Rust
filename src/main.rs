mod types;
mod download;
mod httping;
mod ip;
mod tcping;
mod progress;
mod csv;
mod version;

use anyhow::Result;
use clap::{Command, Arg};
use std::time::Duration;
use crate::types::{Config, DelayFilter};
use crate::csv::PrintResult;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const NAME: &str = "CloudflareST-rust";
const HELP_TEXT: &str = r#"
CloudflareST-rust

参数：
    -n 200
        延迟测速线程；越多延迟测速越快，性能弱的设备 (如路由器) 请勿太高；(默认 200 最多 1000)
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
        测速全部的 IPv4；(IPv4 默认每 /24 段随机测速一个 IP)
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

fn wait_for_input() {
    println!("\n按回车键退出...");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap_or_default();
}

#[tokio::main]
async fn main() {
    match run().await {
        Ok(_) => {
            println!("\n测试完成!");
            wait_for_input();
        }
        Err(e) => {
            eprintln!("\n错误: {}", e);
            if let Some(source) = e.source() {
                eprintln!("详细信息: {}", source);
            }
            wait_for_input();
        }
    }
}

async fn run() -> Result<()> {
    let matches = create_app().get_matches();
    
    if matches.contains_id("v") {
        print_version();
        println!("检查版本更新中...");
        if let Some(new_version) = version::check_update().await {
            println!("*** 发现新版本 [{}]！请前往 [https://github.com/yourusername/CloudflareST-rust] 更新！ ***", new_version);
        } else {
            println!("当前为最新版本 [{}]！", VERSION);
        }
        return Ok(());
    }

    let mut config = Config::from_matches(&matches)?;

    // 设置全局延迟和丢包率限制
    unsafe {
        types::INPUT_MAX_DELAY = config.max_delay;
        types::INPUT_MIN_DELAY = config.min_delay;
        types::INPUT_MAX_LOSS_RATE = config.max_loss_rate;
    }

    println!("CloudflareST-rust {}\n", VERSION);
    
    ip::init_rand_seed();
    check_config(&config);

    let ping = tcping::new_ping(config.clone()).await?;
    let ping_data = ping.run()
        .await?
        .filter_delay()
        .filter_loss_rate();

    let mut speed_data = download::test_download_speed(&mut config, ping_data).await?;
    csv::export_csv(&mut speed_data, &config).await?;
    speed_data.print();

    Ok(())
}

fn create_app() -> Command {
    Command::new("CloudflareST-Rust")
        .version(VERSION)
        .about(HELP_TEXT)
        .disable_version_flag(true)
        .arg(Arg::new("n")
            .value_name("n")
            .value_parser(clap::value_parser!(u32))
            .default_value("200")
            .help("延迟测速线程数"))
        .arg(Arg::new("t")
            .value_name("t")
            .value_parser(clap::value_parser!(u32))
            .default_value("4")
            .help("延迟测速次数"))
        .arg(Arg::new("dn")
            .value_name("dn")
            .value_parser(clap::value_parser!(u32))
            .default_value("10")
            .help("下载测速数量"))
        .arg(Arg::new("dt")
            .value_name("dt")
            .value_parser(clap::value_parser!(u64))
            .default_value("10")
            .help("下载测速时间(秒)"))
        .arg(Arg::new("tp")
            .value_name("tp")
            .value_parser(clap::value_parser!(u16))
            .default_value("443")
            .help("测速端口"))
        .arg(Arg::new("url")
            .value_name("url")
            .value_parser(|s: &str| Ok::<String, String>(s.to_string()))
            .default_value("https://cf.xiu2.xyz/url")
            .help("测速URL"))
        .arg(Arg::new("httping")
            .id("httping")
            .help("切换HTTP测速模式"))
        .arg(Arg::new("httping-code")
            .value_name("httping-code")
            .value_parser(clap::value_parser!(u16))
            .default_value("200")
            .help("HTTP状态码"))
        .arg(Arg::new("cfcolo")
            .value_name("cfcolo")
            .value_parser(clap::value_parser!(String))
            .help("匹配指定地区"))
        .arg(Arg::new("tl")
            .value_name("tl")
            .value_parser(clap::value_parser!(u64))
            .default_value("9999")
            .help("平均延迟上限(ms)"))
        .arg(Arg::new("tll")
            .value_name("tll")
            .value_parser(clap::value_parser!(u64))
            .default_value("0")
            .help("平均延迟下限(ms)"))
        .arg(Arg::new("tlr")
            .value_name("tlr")
            .value_parser(clap::value_parser!(f32))
            .default_value("1.0")
            .help("丢包率上限"))
        .arg(Arg::new("sl")
            .value_name("sl")
            .value_parser(clap::value_parser!(f64))
            .default_value("0.0")
            .help("下载速度下限(MB/s)"))
        .arg(Arg::new("p")
            .value_name("p")
            .value_parser(clap::value_parser!(u32))
            .default_value("10")
            .help("显示结果数量"))
        .arg(Arg::new("f")
            .value_name("f")
            .value_parser(clap::value_parser!(String))
            .default_value("ip.txt")
            .help("IP段数据文件"))
        .arg(Arg::new("ip")
            .value_name("ip")
            .value_parser(clap::value_parser!(String))
            .help("指定IP段数据"))
        .arg(Arg::new("o")
            .value_name("o")
            .value_parser(clap::value_parser!(String))
            .default_value("result.csv")
            .help("输出结果文件"))
        .arg(Arg::new("dd")
            .id("dd")
            .help("禁用下载测速"))
        .arg(Arg::new("all4")
            .id("all4")
            .help("测速全部的 IPv4"))
        .arg(Arg::new("v")
            .id("v")
            .help("显示版本信息"))
        .arg(Arg::new("more6")
            .id("more6")
            .help("测试更多 IPv6 (每个 CIDR 测速 2^18 即 262144 个)"))
        .arg(Arg::new("lots6")
            .id("lots6")
            .help("测试较多 IPv6 (每个 CIDR 测速 2^16 即 65536 个)"))
        .arg(Arg::new("many6")
            .id("many6")
            .help("测试很多 IPv6 (每个 CIDR 测速 2^12 即 4096 个)"))
        .arg(Arg::new("some6")
            .id("some6")
            .help("测试一些 IPv6 (每个 CIDR 测速 2^8 即 256 个)"))
        .arg(Arg::new("many4")
            .id("many4")
            .help("测试一点 IPv4 (每个 CIDR 测速 2^12 即 4096 个)"))
        .arg(Arg::new("v4")
            .value_name("v4")
            .value_parser(clap::value_parser!(String))
            .help("指定 IPv4 测试数量 (2^n±m)"))
        .arg(Arg::new("v6")
            .value_name("v6")
            .value_parser(clap::value_parser!(String))
            .help("指定 IPv6 测试数量 (2^n±m)"))
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