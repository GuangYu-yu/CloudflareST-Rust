mod types;
mod download;
mod httping;
mod ip;
mod tcping;
mod progress;
mod csv;
mod version;

use anyhow::{Result, Context};
use clap::{Command, Arg};
use std::{time::Duration, process};
use crate::types::{Config, DelayFilter};
use crate::csv::PrintResult;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const NAME: &str = "CloudflareST-rust";
const HELP_TEXT: &str = r#"
CloudflareST-rust

参数：
    -n 200
        延迟测速线程数 (默认 200, 最大 1000)
    -t 4
        延迟测速次数 (默认 4)
    -dn 10
        下载测速数量 (默认 10)
    -dt 10
        下载测速时间 (默认 10秒)
    -tp 443
        测速端口 (默认 443)
    -url URL
        测速URL (必需参数)
    -httping
        使用HTTP测速模式
    -tl 200
        平均延迟上限 (默认 9999ms)
    -tll 40
        平均延迟下限 (默认 0ms)
    -tlr 0.2
        丢包率上限 (默认 1.00)
    -sl 5
        下载速度下限 (默认 0.00 MB/s)
    -p 10
        显示结果数量 (默认 10)
    -f ip.txt
        IP段数据文件 (默认 ip.txt)
    -ip IP段数据
        指定IP段数据 (英文逗号分隔)
    -o result.csv
        输出文件 (默认 result.csv)
    -dd
        禁用下载测速
    -allip
        测试所有IP
    -v
        显示版本信息
    -h
        显示帮助信息
"#;

#[tokio::main]
async fn main() -> Result<()> {
    let result = run().await;
    
    // 无论是否出错，都显示错误信息并等待用户输入
    if let Err(e) = result {
        eprintln!("错误: {}", e);
        end_print();
        process::exit(1);
    }
    
    end_print();
    Ok(())
}

async fn run() -> Result<()> {
    let matches = create_app().get_matches();
    
    // 先检查版本参数
    if matches.contains_id("version") {
        print_version();
        println!("检查版本更新中...");
        if let Some(new_version) = version::check_update().await {
            println!("*** 发现新版本 [{}]！请前往 [https://github.com/yourusername/CloudflareST-rust] 更新！ ***", new_version);
        } else {
            println!("当前为最新版本 [{}]！", VERSION);
        }
        return Ok(());
    }

    let mut config = Config::from_matches(&matches)
        .context("解析命令行参数失败")?;

    // 设置全局延迟和丢包率限制
    unsafe {
        types::INPUT_MAX_DELAY = config.max_delay;
        types::INPUT_MIN_DELAY = config.min_delay;
        types::INPUT_MAX_LOSS_RATE = config.max_loss_rate;
    }

    println!("CloudflareST-rust {}\n", VERSION);
    
    // 初始化随机数种子
    ip::init_rand_seed();

    // 检查参数
    check_config(&config);

    // 开始延迟测速 + 过滤延迟/丢包
    let ping = tcping::new_ping(config.clone()).await?;
    let ping_data = ping.run()
        .await?
        .filter_delay()
        .filter_loss_rate();

    let speed_data = download::test_download_speed(&mut config, ping_data).await?;

    // 输出结果
    csv::export_csv(&speed_data, &config)?;
    speed_data.print();

    end_print();
    Ok(())
}

fn create_app() -> Command {
    Command::new("CloudflareST-rust")
        .version(VERSION)
        .about(HELP_TEXT)
        .arg(Arg::new("n")
            .short('n')
            .value_parser(clap::value_parser!(u32))
            .default_value("200")
            .help("延迟测速线程数"))
        .arg(Arg::new("t")
            .short('t')
            .value_parser(clap::value_parser!(u32))
            .default_value("4")
            .help("延迟测速次数"))
        .arg(Arg::new("dn")
            .long("dn")
            .value_parser(clap::value_parser!(u32))
            .default_value("10")
            .help("下载测速数量"))
        .arg(Arg::new("dt")
            .long("dt")
            .value_parser(clap::value_parser!(u64))
            .default_value("10")
            .help("下载测速时间(秒)"))
        .arg(Arg::new("tp")
            .long("tp")
            .value_parser(clap::value_parser!(u16))
            .default_value("443")
            .help("测速端口"))
        .arg(Arg::new("url")
            .long("url")
            .required(false)
            .value_parser(|s: &str| {
                if s.starts_with("http://") || s.starts_with("https://") {
                    Ok(s.to_string())
                } else {
                    Err(String::from("URL必须以http://或https://开头"))
                }
            })
            .default_value("https://cf.xiu2.xyz/url")
            .help("测速URL"))
        .arg(Arg::new("httping")
            .long("httping")
            .help("切换HTTP测速模式"))
        .arg(Arg::new("httping-code")
            .long("httping-code")
            .value_parser(clap::value_parser!(u16))
            .default_value("200")
            .help("HTTP状态码"))
        .arg(Arg::new("cfcolo")
            .long("cfcolo")
            .value_parser(clap::value_parser!(String))
            .help("匹配指定地区"))
        .arg(Arg::new("tl")
            .long("tl")
            .value_parser(clap::value_parser!(u64))
            .default_value("9999")
            .help("平均延迟上限(ms)"))
        .arg(Arg::new("tll")
            .long("tll")
            .value_parser(clap::value_parser!(u64))
            .default_value("0")
            .help("平均延迟下限(ms)"))
        .arg(Arg::new("tlr")
            .long("tlr")
            .value_parser(clap::value_parser!(f32))
            .default_value("1.0")
            .help("丢包率上限"))
        .arg(Arg::new("sl")
            .long("sl")
            .value_parser(clap::value_parser!(f64))
            .default_value("0.0")
            .help("下载速度下限(MB/s)"))
        .arg(Arg::new("p")
            .short('p')
            .value_parser(clap::value_parser!(u32))
            .default_value("10")
            .help("显示结果数量"))
        .arg(Arg::new("f")
            .short('f')
            .value_parser(clap::value_parser!(String))
            .default_value("ip.txt")
            .help("IP段数据文件"))
        .arg(Arg::new("ip")
            .long("ip")
            .value_parser(clap::value_parser!(String))
            .help("指定IP段数据"))
        .arg(Arg::new("o")
            .short('o')
            .value_parser(clap::value_parser!(String))
            .default_value("result.csv")
            .help("输出结果文件"))
        .arg(Arg::new("dd")
            .long("dd")
            .help("禁用下载测速"))
        .arg(Arg::new("allip")
            .long("allip")
            .help("测试所有IP"))
        .arg(Arg::new("version")
            .short('v')
            .help("显示版本信息"))
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

fn end_print() {
    #[cfg(target_os = "windows")]
    {
        use std::io::Write;
        println!("\n按任意键退出...");
        std::io::stdout().flush().unwrap();
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).unwrap();
    }
} 