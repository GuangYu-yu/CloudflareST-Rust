use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

use crate::args::Args;
use crate::common::{Ping as CommonPing, PingData};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) fn error_println(args: std::fmt::Arguments<'_>) {
    eprintln!("\x1b[31;1m[错误]\x1b[0m {args}");
}

pub(crate) fn error_and_exit(args: std::fmt::Arguments<'_>) -> ! {
    error_println(args);
    std::process::exit(1);
}

pub(crate) fn info_println(args: std::fmt::Arguments<'_>) {
    println!("\x1b[36;1m[信息]\x1b[0m {args}");
}

pub(crate) fn warning_println(args: std::fmt::Arguments<'_>) {
    println!("\x1b[33;1m[警告]\x1b[0m {args}");
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

fn create_ping(args: Arc<Args>, sources: Vec<String>, timeout_flag: Arc<AtomicBool>) -> Option<CommonPing> {
    if args.httping.is_some() {
        return httping::new(args, sources, timeout_flag);
    }
    #[cfg(feature = "icmp")]
    if args.icmp_ping {
        return icmp::new(args, sources, timeout_flag);
    }
    Some(tcping::new(args, sources, timeout_flag))
}

#[tokio::main]
async fn main() {
    println!("\x1b[34;1m# CloudflareST-Rust\x1b[0m");

    let args = Arc::new(args::parse_args());

    let sources = ip::collect_ip_sources(&args.ip_text, &args.ip_file);

    pool::init_global_limiter(args.max_threads);

    let timeout_flag = Arc::new(AtomicBool::new(false));

    if let Some(timeout) = args.global_timeout_duration {
        info_println(format_args!(
            "程序执行时间超过 {timeout:?} 后，将提前结算结果并退出"
        ));
        let timeout_flag_clone = timeout_flag.clone();
        // fire-and-forget：超时后置标志位，主循环自行检测退出
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            timeout_flag_clone.store(true, Ordering::SeqCst);
        });
    }

    let Some(ping) = create_ping(args.clone(), sources, timeout_flag.clone()) else {
        error_and_exit(format_args!("创建测速实例失败"));
    };
    let ping_result: Vec<PingData> = ping.run().await;

    let ping_interrupted = timeout_flag.load(Ordering::SeqCst);

    let ping_data = if args.disable_download || ping_result.is_empty() || ping_interrupted {
        let reason = if args.disable_download {
            "已禁用下载测速"
        } else if ping_interrupted {
            "由于全局超时，跳过下载测速"
        } else {
            "延迟测速结果为空，跳过下载测速"
        };
        info_println(format_args!("{reason}"));
        ping_result
    } else {
        let mut download_test = download::DownloadTest::new(&args, ping_result, timeout_flag.clone()).await;
        download_test.test_download_speed().await
    };

    csv::print_results(&ping_data, &args);

    if let Some(output_file) = &args.output && !ping_data.is_empty() {
        match csv::export_csv(&ping_data, &args) {
            Ok(_) => info_println(format_args!("测速结果已写入 {output_file} 文件，可使用记事本/表格软件查看")),
            Err(e) => info_println(format_args!("导出 CSV 失败: {e:?}")),
        }
    }

    info_println(format_args!("CloudflareST-Rust 执行完毕"));
}