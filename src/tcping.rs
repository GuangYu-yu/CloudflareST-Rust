use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::net::TcpStream;
use std::io;
use futures::stream::{FuturesUnordered, StreamExt};
use std::sync::atomic::{AtomicUsize, AtomicBool, Ordering};

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
    success_count: Arc<AtomicUsize>,
    timeout_flag: Arc<AtomicBool>,
}

impl Ping {
    pub async fn new(args: &Args, timeout_flag: Arc<AtomicBool>) -> io::Result<Self> {
        let (ip_buffer, csv, bar, max_loss_rate) = common::init_ping_test(args);

        Ok(Ping {
            ip_buffer,
            csv,
            bar,
            max_loss_rate,
            args: args.clone(),
            success_count: Arc::new(AtomicUsize::new(0)),
            timeout_flag,
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
        let timeout_flag = Arc::clone(&self.timeout_flag);

        // 使用FuturesUnordered来动态管理任务
        let mut tasks = FuturesUnordered::new();
        
        // 获取当前线程池的并发能力
        let initial_tasks = GLOBAL_POOL.get_concurrency_level();

        let add_task = |ip: IpAddr, tasks: &mut FuturesUnordered<_>| {
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
        };

        // 初始填充任务队列
        for _ in 0..initial_tasks {
            let ip = {
                let mut ip_buffer = ip_buffer.lock().unwrap();
                ip_buffer.pop()
            };
            
            if let Some(ip) = ip {
                add_task(ip, &mut tasks);
            } else {
                break;
            }
        }

        // 动态处理任务完成和添加新任务
        while let Some(result) = tasks.next().await {
            // 检查是否收到超时信号
            if common::check_timeout_signal(&timeout_flag) {
                break;
            }
            
            // 处理已完成的任务
            let _ = result;
            
            // 添加新任务
            let ip = {
                let mut ip_buffer = ip_buffer.lock().unwrap();
                ip_buffer.pop()
            };
            
            if let Some(ip) = ip {
                add_task(ip, &mut tasks);
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

// TCP 测速处理函数
async fn tcping_handler(
    ip: IpAddr, 
    csv: Arc<Mutex<PingDelaySet>>, 
    bar: Arc<Bar>, 
    args: &Args,
    success_count: Arc<AtomicUsize>,
) {
    let ping_times = common::get_ping_times(args);
    let mut tasks = FuturesUnordered::new();

    // 使用FuturesUnordered来动态管理并发测试
    for _ in 0..ping_times {
        let args_clone = args.clone();
        tasks.push(tokio::spawn(async move {
            let port = common::get_tcp_port(&args_clone);
            let addr = SocketAddr::new(ip, port);
            tcping(addr, &args_clone).await
        }));
    }

    // 处理并发结果
    let mut recv = 0;
    let mut total_delay_ms = 0.0;
    
    while let Some(result) = tasks.next().await {
        if let Ok(Some(delay)) = result {
            recv += 1;
            total_delay_ms += delay;
        }
    }

    // 计算平均延迟
    let avg_delay_ms = common::calculate_precise_delay(total_delay_ms, recv);

    if recv > 0 {
        success_count.fetch_add(1, Ordering::Relaxed);
        let data = PingData::new(ip, ping_times, recv, avg_delay_ms);
        
        if common::should_keep_result(&data, args) {
            let mut csv_guard = csv.lock().unwrap();
            csv_guard.push(data);
        }
    }
    
    // 无论成功与否，每个IP测试完成后都增加1个计数
    let current_count = success_count.load(Ordering::Relaxed);
    bar.grow(1, current_count.to_string());
}

// TCP连接测试函数
async fn tcping(addr: SocketAddr, args: &Args) -> Option<f32> {
    // 开始任务
    GLOBAL_POOL.start_task();
    
    // 创建CPU计时器
    let mut cpu_timer = GLOBAL_POOL.start_cpu_timer();
    
    let connect_result = tokio::time::timeout(
        args.max_delay,
        async {
        let start_time = Instant::now();
        
        // 暂停CPU计时(网络连接阶段)
        cpu_timer.pause();
        let stream_result = TcpStream::connect(&addr).await;
        // 恢复CPU计时(结果处理阶段)
        cpu_timer.resume();
        
        match stream_result {
            Ok(stream) => {
                // 结果处理(关闭连接等操作)在计时范围内
                let _ = stream.set_linger(None);
                drop(stream);
                cpu_timer.finish();
                Some(start_time.elapsed().as_secs_f32() * 1000.0)
            },
            Err(_) => None
        }
    }).await;
    
    // 结束任务
    GLOBAL_POOL.end_task();
    
    connect_result.unwrap_or(None)
}