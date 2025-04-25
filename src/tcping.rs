use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use std::io;
use futures::stream::{FuturesUnordered, StreamExt};

use crate::progress::Bar;
use crate::args::Args;
use crate::pool::{execute_with_rate_limit, GLOBAL_POOL};
use crate::common::{self, PingData, PingDelaySet};
use crate::ip::IpBuffer;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct Ping {
    ip_buffer: Arc<Mutex<IpBuffer>>,
    csv: Arc<Mutex<PingDelaySet>>,
    bar: Arc<Bar>,
    max_loss_rate: f32,
    args: Args,
    success_count: Arc<AtomicUsize>,
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
            success_count: Arc::new(AtomicUsize::new(0)),
        })
    }

    pub async fn run(self) -> Result<PingDelaySet, io::Error> {
        // 检查IP缓冲区是否为空
        {
            let ip_buffer = self.ip_buffer.lock().unwrap();
            if ip_buffer.is_empty() && ip_buffer.total_expected() == 0 {
                return Ok(Vec::new());
            }
        }

        // 打印开始延迟测试的信息
        common::print_speed_test_info(
            "Tcping",
            common::get_tcp_port(&self.args),
            common::get_min_delay(&self.args),
            common::get_max_delay(&self.args),
            self.max_loss_rate
        );

        // 准备工作数据
        let ip_buffer = Arc::clone(&self.ip_buffer);
        let csv = Arc::clone(&self.csv);
        let bar = Arc::clone(&self.bar);
        let args = self.args.clone();
        let success_count = Arc::clone(&self.success_count);
        
        // 使用FuturesUnordered来动态管理任务
        let mut tasks = FuturesUnordered::new();
        
        // 获取当前线程池的并发能力
        let initial_tasks = GLOBAL_POOL.get_concurrency_level();
        
        // 初始填充任务队列
        for _ in 0..initial_tasks {
            let ip = {
                let mut ip_buffer = ip_buffer.lock().unwrap();
                ip_buffer.pop()
            };
            
            if let Some(ip) = ip {
                let csv_clone = Arc::clone(&csv);
                let bar_clone = Arc::clone(&bar);
                let args_clone = args.clone();
                let success_count_clone = Arc::clone(&success_count);
                
                tasks.push(tokio::spawn(async move {
                    execute_with_rate_limit(|| async move {
                        tcping_handler(ip, csv_clone, bar_clone, &args_clone, success_count_clone).await;
                        Ok::<(), io::Error>(())
                    }).await.unwrap();
                }));
            } else {
                break;
            }
        }
        
        // 动态处理任务完成和添加新任务
        while let Some(result) = tasks.next().await {
            // 处理已完成的任务
            let _ = result;
            
            // 添加新任务
            let ip = {
                let mut ip_buffer = ip_buffer.lock().unwrap();
                ip_buffer.pop()
            };
            
            if let Some(ip) = ip {
                let csv_clone = Arc::clone(&csv);
                let bar_clone = Arc::clone(&bar);
                let args_clone = args.clone();
                let success_count_clone = Arc::clone(&success_count);
                
                tasks.push(tokio::spawn(async move {
                    execute_with_rate_limit(|| async move {
                        tcping_handler(ip, csv_clone, bar_clone, &args_clone, success_count_clone).await;
                        Ok::<(), io::Error>(())
                    }).await.unwrap();
                }));
            }
        }

        // 更新进度条为完成状态
        self.bar.done();

        // 收集所有测试结果，排序后返回
        let mut results = self.csv.lock().unwrap().clone();
        
        // 使用common模块的排序函数
        common::sort_ping_results(&mut results);
        
        Ok(results)
    }
}

pub async fn tcping(ip: IpAddr, args: &Args) -> Option<f32> {
    // 使用GLOBAL_POOL获取任务ID
    let task_id = GLOBAL_POOL.get_task_id();
    GLOBAL_POOL.start_task(task_id);
    
    // 记录开始连接时间
    let start_time = Instant::now();
    
    // 尝试建立TCP连接
    let port = common::get_tcp_port(args);
    let addr = SocketAddr::new(ip, port);
    
    // 使用更安全的方式处理连接和超时
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    
    // 在连接尝试循环中使用select!宏同时处理连接和心跳
    let connect_result = tokio::time::timeout(args.max_delay, async {
        loop {
            tokio::select! {
                // 尝试连接
                result = TcpStream::connect(&addr) => {
                    match result {
                        Ok(stream) => return Some((stream, start_time.elapsed().as_secs_f32() * 1000.0)),
                        Err(_) => {}  // 连接失败，继续尝试
                    }
                },
                _ = interval.tick() => {
                    // 记录心跳
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
    
    // 结束任务
    GLOBAL_POOL.end_task(task_id);
    
    result
}

// 处理单个IP的TCP测速
async fn tcping_handler(
    ip: IpAddr, 
    csv: Arc<Mutex<PingDelaySet>>, 
    bar: Arc<Bar>, 
    args: &Args,
    success_count: Arc<AtomicUsize>,
) {
    // 执行连接测试
    let (recv, avg_delay_ms) = check_connection(ip, args).await;
    
    if recv == 0 {
        // 连接失败，更新进度条（使用成功连接数）
        let current_success = success_count.load(Ordering::Relaxed);
        bar.grow(1, current_success.to_string());
        return;
    }

    // 连接成功，增加成功计数
    let new_success_count = success_count.fetch_add(1, Ordering::Relaxed) + 1;
    
    // 创建测试数据
    let ping_times = common::get_ping_times(args);
    let data = PingData::new(ip, ping_times, recv, avg_delay_ms);
    
    // 应用筛选条件（但不影响进度条计数）
    if common::should_keep_result(&data, args) {
        let mut csv_guard = csv.lock().unwrap();
        csv_guard.push(data);
    }
    
    // 更新进度条（使用成功连接数）
    bar.grow(1, new_success_count.to_string());
}

// 执行连接测试
async fn check_connection(ip: IpAddr, args: &Args) -> (u16, f32) {
    let ping_times = common::get_ping_times(args);
    
    // 创建多个并发任务
    let mut tasks = Vec::with_capacity(ping_times as usize);
    for _ in 0..ping_times {
        tasks.push(tcping(ip, args));
    }
    
    // 并行等待所有任务完成
    let results = futures::future::join_all(tasks).await;
    
    // 处理结果
    let mut recv = 0;
    let mut total_delay_ms = 0.0;
    
    for result in results {
        if let Some(delay_ms) = result {
            recv += 1;
            total_delay_ms += delay_ms;
        }
    }
    
    // 使用 common 模块中的函数计算延迟
    let avg_delay_ms = common::calculate_precise_delay(total_delay_ms, recv);
    
    (recv, avg_delay_ms)
}
