mod types;
mod download;
mod httping;
mod ip;
mod tcping;
mod progress;
mod csv;
mod version;

use anyhow::{Result, Context};
use clap::{App, Arg};
use std::{time::Duration, process};
use crate::types::{Config, PingDelaySet};
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
"#;

#[tokio::main]
async fn main() -> Result<()> {
    if let Err(e) = run().await {
        eprintln!("错误: {}", e);
        process::exit(1);
    }
    Ok(())
}

async fn run() -> Result<()> {
    let matches = create_app().get_matches();
    
    // 先检查版本参数
    if matches.is_present("version") {
        print_version();
        println!("检查版本更新中...");
        if let Some(new_version) = version::check_update().await {
            println!("*** 发现新版本 [{}]！请前往 [https://github.com/yourusername/CloudflareST-rust] 更新！ ***", new_version);
        } else {
            println!("当前为最新版本 [{}]！", VERSION);
        }
        return Ok(());
    }

    let config = Config::from_matches(&matches)
        .context("解析命令行参数失败")?;

    println!("CloudflareST-rust {}\n", VERSION);
    
    // 初始化随机数种子
    ip::init_rand_seed();

    // 检查参数
    check_config(&config);

    // 开始延迟测速
    let ping_data = tcping::new_ping(&config)
        .run()
        .await?;


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
            .validator(|v| v.parse::<u32>().map(|_| ()).map_err(|e| e.to_string()))
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
            .required(true)
            .validator(|v| {
                if v.starts_with("http://") || v.starts_with("https://") {
                    Ok(())
                } else {
                    Err("URL必须以http://或https://开头".to_string())
                }
            })
            .help("测速URL(必需)"))
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
            .help("测试所有IP"))
        .arg(Arg::with_name("version")
            .short("v")
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