mod args;
mod httping;
mod tcping;
// mod icmp;
mod common;
mod csv;
mod download;
mod ip;
mod pool;
mod progress;

use crate::common::PingData;
use crate::csv::PrintResult;
use fastrand;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

// 将 main 函数修改为
#[tokio::main]
async fn main() {
    // 解析命令行参数
    let args = args::parse_args();

    // 初始化全局并发限制器
    pool::init_global_limiter(args.max_threads);

    // 创建全局超时标志
    let timeout_flag = Arc::new(AtomicBool::new(false));

    // 设置全局超时
    if let Some(timeout) = args.global_timeout_duration {
        println!(
            "\n[信息] 程序执行时间超过 {:?} 后，将提前结算结果并退出",
            timeout
        );
        let timeout_flag_clone = Arc::clone(&timeout_flag);
        thread::spawn(move || {
            thread::sleep(timeout);
            timeout_flag_clone.store(true, Ordering::SeqCst);
        });
    }

    // 初始化随机数种子
    let _ = fastrand::u32(..);

    println!("# CloudflareST-Rust\n");

    // 根据参数选择 TCP、HTTP 或 ICMP 测速
    let ping_result: Vec<PingData> = if args.httping {
        httping::Ping::new(&args, Arc::clone(&timeout_flag))
            .await
            .unwrap()
            .run()
            .await
            .unwrap()
    }
    /* else if args.icmp_ping {
        icmp::Ping::new(&args, Arc::clone(&timeout_flag))
            .await.unwrap()
            .run().await.unwrap()
    } */
    else {
        tcping::Ping::new(&args, Arc::clone(&timeout_flag))
            .await
            .unwrap()
            .run()
            .await
            .unwrap()
    };

    // 检查是否在 ping 阶段被超时中断
    let ping_interrupted = timeout_flag.load(Ordering::SeqCst);

    // 开始下载测速
    let ping_data = if args.disable_download || ping_result.is_empty() || ping_interrupted {
        println!(
            "\n[信息] {}",
            if args.disable_download {
                "已禁用下载测速"
            } else if ping_interrupted {
                "由于全局超时，跳过下载测速"
            } else {
                "延迟测速结果为空，跳过下载测速"
            }
        );
        ping_result
    } else {
        // 创建可变下载测速实例
        let mut download_test =
            download::DownloadTest::new(&args, ping_result, Arc::clone(&timeout_flag)).await;

        // 执行下载测速
        download_test.test_download_speed().await
    };

    // 输出文件
    if let Err(e) = csv::export_csv(&ping_data, &args) {
        println!("\n[信息] 导出CSV失败: {:?}", e);
    }

    // 打印结果
    ping_data.print(&args);

    println!("程序执行完毕");
}