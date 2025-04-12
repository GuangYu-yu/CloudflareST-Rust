use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
static TASK_ID_COUNTER: AtomicUsize = AtomicUsize::new(0);

use crate::progress::Bar;
use crate::args::Args;
use crate::pool::{execute_with_rate_limit, GLOBAL_POOL};
use crate::common::{self, PingData, PingDelaySet};
use crate::ip::IpBuffer;

pub struct Ping {
    ip_buffer: Arc<Mutex<IpBuffer>>,
    csv: Arc<Mutex<PingDelaySet>>,
    bar: Arc<Bar>,
    max_loss_rate: f32,
    args: Args,
}

impl Ping {
    pub async fn new(args: &Args) -> io::Result<Self> {
        let (ip_buffer, csv, bar, max_loss_rate) = common::init_ping_test(args);
        
        Ok(Ping {
            ip_buffer,
            csv,
            bar,
            max_loss_rate,
            args: args.clone(),
        })
    }

    pub async fn run(self) -> Result<PingDelaySet, io::Error> {
        // 1. 检查IP缓冲区是否为空
        {
            let ip_buffer = self.ip_buffer.lock().unwrap();
            if ip_buffer.is_empty() && ip_buffer.total_expected() == 0 {
                return Ok(Vec::new());
            }
        }

        // 2. 打印开始延迟测试的信息
        common::print_speed_test_info(
            "Tcping",
            common::get_tcp_port(&self.args),
            common::get_min_delay(&self.args),
            common::get_max_delay(&self.args),
            self.max_loss_rate
        );

        // 3. 创建线程安全集合和任务句柄向量
        let mut handles = Vec::new();

        // 4. 循环从IP缓冲区获取IP并启动测试任务
        loop {
            // 从缓冲区获取IP
            let ip = {
                let mut ip_buffer = self.ip_buffer.lock().unwrap();
                ip_buffer.pop()
            };
            
            // 如果没有更多IP，退出循环
            if ip.is_none() {
                break;
            }
            
            let ip = ip.unwrap();
            let csv = Arc::clone(&self.csv);
            let bar = Arc::clone(&self.bar);
            let args = self.args.clone();

            // 创建异步任务
            let handle = tokio::spawn(async move {
                // 申请全局并发控制许可
                execute_with_rate_limit(|| async {
                    tcping_handler(ip, csv, bar, &args).await;
                    Ok::<(), io::Error>(())
                }).await.unwrap();
            });

            handles.push(handle);
        }

        // 5. 等待所有剩余的异步任务完成
        for handle in handles {
            let _ = handle.await;
        }

        // 6. 更新进度条为完成状态
        self.bar.done();

        // 收集所有测试结果，排序后返回
        let mut results = self.csv.lock().unwrap().clone();
        
        // 按延迟排序
        results.sort_by(|a, b| a.delay.partial_cmp(&b.delay).unwrap_or(std::cmp::Ordering::Equal));
        
        Ok(results)
    }
}

pub async fn tcping(ip: IpAddr, args: &Args) -> Option<f64> {
    // 1. 生成唯一任务标识并记录任务开始
    let task_id = TASK_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
    GLOBAL_POOL.start_task(task_id);
    
    // 2. 记录开始连接时间
    let start_time = Instant::now();
    
    // 3. 尝试建立TCP连接
    let port = common::get_tcp_port(args);
    let addr = SocketAddr::new(ip, port);
    
    // 使用更安全的方式处理连接和超时
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    
    // 在连接尝试循环中使用common中的心跳功能
    let connect_result = tokio::time::timeout(args.max_delay, async {
        loop {
            // 尝试连接
            match TcpStream::connect(&addr).await {
                Ok(stream) => return Some((stream, start_time.elapsed().as_secs_f64() * 1000.0)),
                Err(_) => {
                    // 等待下一个间隔
                    interval.tick().await;
                    // 记录进度
                    GLOBAL_POOL.record_progress(task_id);
                }
            }
        }
    }).await;
    
    // 处理连接结果
    let result = match connect_result {
        Ok(Some((stream, duration_ms))) => {
            // 连接成功
            drop(stream);
            Some(duration_ms)
        },
        _ => None, // 连接超时或失败
    };
    
    // 4. 结束任务
    GLOBAL_POOL.end_task(task_id);
    
    result
}

// 处理单个IP的TCP测速
async fn tcping_handler(
    ip: IpAddr, 
    csv: Arc<Mutex<PingDelaySet>>, 
    bar: Arc<Bar>, 
    args: &Args,
) {
    // 执行连接测试
    let (recv, avg_delay_ms) = check_connection(ip, args).await;
    
    // 获取当前可用IP数量
    let now_able = {
        let csv_guard = csv.lock().unwrap();
        csv_guard.len()
    };
    
    // 更新进度条
    bar.grow(1, now_able.to_string());
    
    // 如果没有成功连接，直接返回（IP会被自动回收）
    if recv == 0 {
        return;
    }
    
    // 创建测试数据
    let ping_times = common::get_ping_times(args);
    let data = PingData::new(ip, ping_times, recv, avg_delay_ms);
    
    // 应用筛选条件
    if common::should_keep_result(&data, args) {
        // 添加到结果集
        let mut csv_guard = csv.lock().unwrap();
        csv_guard.push(data);
    }
    // 如果不符合筛选条件，IP会被自动回收
}

// 执行连接测试
async fn check_connection(ip: IpAddr, args: &Args) -> (usize, f64) {
    let mut recv = 0;
    let mut total_delay_ms = 0.0;
    let ping_times = common::get_ping_times(args);
    
    for _ in 0..ping_times {
        if let Some(delay_ms) = tcping(ip, args).await {
            recv += 1;
            total_delay_ms += delay_ms;
        }
    }
    
    // 使用 common 模块中的函数计算延迟
    let avg_delay_ms = common::calculate_precise_delay(total_delay_ms, recv);
    
    (recv, avg_delay_ms)
}
