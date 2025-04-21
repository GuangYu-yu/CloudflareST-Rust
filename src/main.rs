mod args;
mod tcping;
mod httping;
mod download;
mod csv;
mod progress;
mod ip;
mod pool;
mod common;

use std::thread;
use rand::prelude::*;
use crate::csv::PrintResult;
use crate::common::PingData;

// 将 main 函数修改为
#[tokio::main]
async fn main() {
    // 解析命令行参数
    let args = args::parse_args();
    
    // 设置全局超时
    if let Some(timeout) = args.global_timeout_duration {
        // 克隆 global_timeout 字符串
        let timeout_str = args.global_timeout.clone();
        thread::spawn(move || {
            thread::sleep(timeout);
            println!("\n程序已达到设定的超时时间 {}，自动退出", timeout_str);
            std::process::exit(0);
        });
    }
    
    // 初始化随机数种子
    init_rand_seed();
    
    println!("# CloudflareST-Rust\n");
    
    // 根据参数选择 TCP 或 HTTP 测速
    let ping_result: Vec<PingResult> = if args.httping {
        httping::Ping::new(&args).await.unwrap().run().await.unwrap()
            .into_iter().map(PingResult::Http).collect()
    } else {
        tcping::Ping::new(&args).await.unwrap().run().await.unwrap()
            .into_iter().map(PingResult::Tcp).collect()
    };
    
    // 开始下载测速
    let (result, no_qualified) = if args.disable_download || ping_result.is_empty() {
        println!("\n[信息] {}", if args.disable_download { "已禁用下载测速" } else { "延迟测速结果为空，跳过下载测速" });
        (ping_result, false)
    } else {
        // 创建可变下载测速实例
        let mut download_test = download::DownloadTest::new(&args, ping_result).await;
        
        // 执行下载测速
        download_test.test_download_speed().await
    };
    
    // 输出文件
    if let Err(e) = csv::export_csv(&result, &args) {
        println!("\n[信息] 导出CSV失败: {:?}", e);
    }
    
    // 打印结果
    result.print(&args, no_qualified);
    
    println!("程序执行完毕");
}

// 初始化随机数种子
fn init_rand_seed() {
    let mut rng = rand::rng();
    let _: u32 = rng.random();
}

// 定义一个枚举来封装不同的 PingData 类型
#[derive(Clone, Debug)]
pub enum PingResult {
    Tcp(PingData),
    Http(PingData),
}