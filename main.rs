mod types;
mod download;
mod httping;
mod ip;
mod tcping;
mod progress;
mod csv;

use anyhow::{Result, Context};
use clap::{App, Arg};
use std::{time::Duration, process};
use crate::types::{Config, PingDelaySet};

const VERSION: &str = "0.1.0";
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
    -url URL
        指定测速地址；延迟测速(HTTPing)/下载测速时使用的地址
    -httping
        切换测速模式；延迟测速模式改为 HTTP 协议
    -tl 200
        平均延迟上限；只输出低于指定平均延迟的 IP；(��认 9999 ms)
    -tll 40
        平均延迟下限；只输出高于指定平均延迟的 IP；(默认 0 ms)
    -tlr 0.2
        丢包几率上限；只输出低于/等于指定丢包率的 IP；(默认 1.00)
    -sl 5
        下载速度下限；只输出高于指定下载速度的 IP；(默认 0.00 MB/s)
    -p 10
        显示结果数量；(默认 10 个)
    -f ip.txt
        IP段数据文件；(默认 ip.txt)
    -ip IP段数据
        指定IP段数据；直接通过参数指定要测速的 IP 段数据，英文逗号分隔
    -o result.csv
        写入结果文件；(默认 result.csv)
    -dd
        禁用下载测速；禁用后测速结果会按延迟排序
    -allip
        测速全部的IP；对 IP 段中的每个 IP 进行测速
"#;

#[tokio::main]
async fn main() -> Result<()> {
    if let Err(e) = run().await {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
    Ok(())
}

async fn run() -> Result<()> {
    let matches = create_app().get_matches();
    let config = Config::from_matches(&matches)
        .context("Failed to parse command line arguments")?;
    
    if matches.is_present("version") {
        print_version();
        return Ok(());
    }

    println!("CloudflareST-rust {}\n", VERSION);

    // 初始化随机数种子
    ip::init_rand_seed();

    // 检查参数
    check_config(&config);

    // 开始延迟测速
    let ping_data = tcping::new_ping(&config)
        .run()
        .await?
        .filter_delay()
        .filter_loss_rate();

    // 提示信息
    print_test_info(&config, &ping_data);

    // 开始下载测速
    if !config.disable_download {
        println!(
            "开始下载测速（下限：{:.2} MB/s, 数量：{}, 队列：{}）",
            config.min_speed,
            config.test_count,
            ping_data.len().min(config.test_count as usize)
        );
    }

    let speed_data = download::test_download_speed(&config, ping_data).await?;

    // 输出结果
    csv::export_csv(&speed_data, &config)?;
    speed_data.print();

    end_print(&config);
    Ok(())
}

fn create_app() -> App<'static, 'static> {
    App::new("CloudflareST-rust")
        .version(VERSION)
        .about("测试 Cloudflare CDN 所有 IP 的延迟和速度")
        .arg(Arg::with_name("n")
            .short("n")
            .takes_value(true)
            .default_value("200")
            .validator(|v| v.parse::<u32>()
                .map(|n| if n > 1000 { 
                    Err("线程数不能超过1000".to_string())
                } else {
                    Ok(())
                })
                .map_err(|e| e.to_string())?)
            .help("延迟测速线程数"))
        .arg(Arg::with_name("t")
            .short("t")
            .takes_value(true)
            .default_value("4")
            .validator(|v| v.parse::<u32>().map(|_| ()).map_err(|e| e.to_string()))
            .help("延迟测速次数"))
        .arg(Arg::with_name("dn")
            .long("dn")
            .takes_value(true)
            .default_value("10")
            .validator(|v| v.parse::<u32>().map(|_| ()).map_err(|e| e.to_string()))
            .help("下载测速数量"))
        .arg(Arg::with_name("dt")
            .long("dt")
            .takes_value(true)
            .default_value("10")
            .validator(|v| v.parse::<u64>().map(|_| ()).map_err(|e| e.to_string()))
            .help("下载测速时间(秒)"))
        .arg(Arg::with_name("tp")
            .long("tp")
            .takes_value(true)
            .default_value("443")
            .validator(|v| v.parse::<u16>().map(|_| ()).map_err(|e| e.to_string()))
            .help("测速端口"))
        .arg(Arg::with_name("url")
            .long("url")
            .takes_value(true)
            .help("测速URL"))
        .arg(Arg::with_name("httping")
            .long("httping")
            .help("切换HTTP测速模式"))
        .arg(Arg::with_name("httping-code")
            .long("httping-code")
            .takes_value(true)
            .default_value("200")
            .validator(|v| v.parse::<u16>().map(|_| ()).map_err(|e| e.to_string()))
            .help("HTTP状态码"))
        .arg(Arg::with_name("cfcolo")
            .long("cfcolo")
            .takes_value(true)
            .help("匹配指定地区"))
        .arg(Arg::with_name("tl")
            .long("tl")
            .takes_value(true)
            .default_value("9999")
            .validator(|v| v.parse::<u64>().map(|_| ()).map_err(|e| e.to_string()))
            .help("平均延迟上限(ms)"))
        .arg(Arg::with_name("tll")
            .long("tll")
            .takes_value(true)
            .default_value("0")
            .validator(|v| v.parse::<u64>().map(|_| ()).map_err(|e| e.to_string()))
            .help("平均延迟下限(ms)"))
        .arg(Arg::with_name("tlr")
            .long("tlr")
            .takes_value(true)
            .default_value("1.0")
            .validator(|v| v.parse::<f32>()
                .map(|n| if n < 0.0 || n > 1.0 {
                    Err("丢包率必须在0.0-1.0之间".to_string())
                } else {
                    Ok(())
                })
                .map_err(|e| e.to_string())?)
            .help("丢包率上限"))
        .arg(Arg::with_name("sl")
            .long("sl")
            .takes_value(true)
            .default_value("0.0")
            .validator(|v| v.parse::<f64>().map(|_| ()).map_err(|e| e.to_string()))
            .help("下载速度下限(MB/s)"))
        .arg(Arg::with_name("p")
            .short("p")
            .takes_value(true)
            .default_value("10")
            .validator(|v| v.parse::<u32>().map(|_| ()).map_err(|e| e.to_string()))
            .help("显示结果数量"))
        .arg(Arg::with_name("f")
            .short("f")
            .takes_value(true)
            .default_value("ip.txt")
            .help("IP段数据文件"))
        .arg(Arg::with_name("ip")
            .long("ip")
            .takes_value(true)
            .help("指定IP段数据"))
        .arg(Arg::with_name("o")
            .short("o")
            .takes_value(true)
            .default_value("result.csv")
            .help("输出结果文件"))
        .arg(Arg::with_name("dd")
            .long("dd")
            .help("禁用下载测速"))
        .arg(Arg::with_name("allip")
            .long("allip")
            .help("测速全部的IP"))
        .arg(Arg::with_name("version")
            .short("v")
            .help("显示版本信息"))
}

fn check_config(config: &Config) {
    if config.min_speed > 0.0 && config.max_delay == Duration::from_millis(9999) {
        println!("[小提示] 在使用 [-sl] 参数时，建议搭配 [-tl] 参数，以避免因凑不够 [-dn] 数量而一直测速...");
    }
}

fn print_version() {
    println!("CloudflareST-rust v{}", VERSION);
    
    #[cfg(debug_assertions)]
    println!("Debug Build");
    
    #[cfg(not(debug_assertions))]
    println!("Release Build");
}

fn print_test_info(config: &Config, ping_data: &PingDelaySet) {
    if config.httping {
        println!(
            "开始延迟测速（模式：HTTP, 端口：{}, 范围：{} ~ {} ms, 丢包：{:.2}）",
            config.tcp_port,
            config.min_delay.as_millis(),
            config.max_delay.as_millis(),
            config.max_loss_rate
        );
    } else {
        println!(
            "开始延迟测速（模式：TCP, 端口：{}, 范围：{} ~ {} ms, 丢包：{:.2}）",
            config.tcp_port,
            config.min_delay.as_millis(),
            config.max_delay.as_millis(),
            config.max_loss_rate
        );
    }

    if ping_data.is_empty() {
        println!("\n[信息] 延迟测速结果 IP 数量为 0，跳过下载测速。");
    }
}

fn end_print(config: &Config) {
    if config.print_num == 0 {
        return;
    }
    
    #[cfg(target_os = "windows")]
    {
        use std::io::Write;
        println!("按任意键退出...");
        std::io::stdout().flush().unwrap();
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).unwrap();
    }
} 