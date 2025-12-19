use crate::common::PingData;
use crate::csv::PrintResult;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

// 定义统一的错误、信息和警告输出函数
pub(crate) fn error_println(args: std::fmt::Arguments<'_>) {
    // 红色加粗
    eprintln!("\x1b[31;1m[错误]\x1b[0m {}", args);
}

pub(crate) fn error_and_exit(args: std::fmt::Arguments<'_>) -> ! {
    error_println(args);
    std::process::exit(1);
}

pub(crate) fn info_println(args: std::fmt::Arguments<'_>) {
    // 青色加粗
    println!("\x1b[36;1m[信息]\x1b[0m {}", args);
}

pub(crate) fn warning_println(args: std::fmt::Arguments<'_>) {
    // 黄色加粗
    println!("\x1b[33;1m[警告]\x1b[0m {}", args);
}

mod args;
mod httping;
mod tcping;
#[cfg(feature = "icmp")]
mod icmp;
mod common;
mod csv;
mod download;
mod hyper;
mod interface;
mod ip;
mod pool;
mod progress;

#[tokio::main]
async fn main() {
    // 打印全局标题
    // 蓝色加粗
    println!("\x1b[34;1m# CloudflareST-Rust\x1b[0m");

    // 解析命令行参数
    let args = args::parse_args();

    // 收集并验证
    let sources = ip::collect_ip_sources(&args.ip_text, &args.ip_url, &args.ip_file).await;

    // 初始化全局并发限制器
    pool::init_global_limiter(args.max_threads);

    // 创建全局超时标志
    let timeout_flag = Arc::new(AtomicBool::new(false));

    // 设置全局超时
    if let Some(timeout) = args.global_timeout_duration {
        info_println(format_args!(
            "程序执行时间超过 {:?} 后，将提前结算结果并退出",
            timeout
        ));
        let timeout_flag_clone = Arc::clone(&timeout_flag);
        thread::spawn(move || {
            thread::sleep(timeout);
            timeout_flag_clone.store(true, Ordering::SeqCst);
        });
    }

    // 根据参数选择 TCP、HTTP 或 ICMP 测速
    let ping_result: Vec<PingData> = match args.httping || args.httping_https {
        true => {
            let ping = httping::new(&args, sources.clone(), Arc::clone(&timeout_flag)).unwrap();
            ping.run().await.unwrap()
        },
        #[cfg(feature = "icmp")]
        false if args.icmp_ping => {
            let ping = icmp::new(&args, sources.clone(), Arc::clone(&timeout_flag)).unwrap();
            ping.run().await.unwrap()
        },
        _ => {
            let ping = tcping::new(&args, sources.clone(), Arc::clone(&timeout_flag)).unwrap();
            ping.run().await.unwrap()
        }
    };

    // 检查是否在 ping 阶段被超时中断
    let ping_interrupted = timeout_flag.load(Ordering::SeqCst);

    // 开始下载测速
    let ping_data = if args.disable_download || ping_result.is_empty() || ping_interrupted {
        let reason = if args.disable_download {
            "已禁用下载测速"
        } else if ping_interrupted {
            "由于全局超时，跳过下载测速"
        } else {
            "延迟测速结果为空，跳过下载测速"
        };
        info_println(format_args!("{}", reason));
        ping_result
    } else {
        // 创建可变下载测速实例
        let mut download_test = download::DownloadTest::new(&args, ping_result, Arc::clone(&timeout_flag)).await;
        // 执行下载测速
        download_test.test_download_speed().await
    };

    // 打印结果
    ping_data.print(&args);

    // 输出文件
    if let Some(output_file) = &args.output && !ping_data.is_empty() {
        match csv::export_csv(&ping_data, &args) {
            Ok(_) => info_println(format_args!("测速结果已写入 {} 文件，可使用记事本/表格软件查看", output_file)),
            Err(e) => info_println(format_args!("导出 CSV 失败: {:?}", e)),
        }
    }

    info_println(format_args!("CloudflareST-Rust 执行完毕"));
}