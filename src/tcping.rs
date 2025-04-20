use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use std::io;

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
        
        // 流控信号量 (控制预取IP的数量)
        let ip_fetch_semaphore = Arc::new(tokio::sync::Semaphore::new(2048));

        // 用于收集所有任务的 JoinHandle
        let mut handles = Vec::new();

        loop {
            // 获取取IP的许可
            let permit = Arc::clone(&ip_fetch_semaphore).acquire_owned().await.unwrap();

            let ip = {
                let mut ip_buffer = ip_buffer.lock().unwrap();
                ip_buffer.pop()
            };

            if ip.is_none() {
                drop(permit);
                break;
            }

            let ip = ip.unwrap();
            let csv_clone = Arc::clone(&csv);
            let bar_clone = Arc::clone(&bar);
            let args_clone = args.clone();
            let sem_clone = Arc::clone(&ip_fetch_semaphore);

            // 并发提交任务，不等待每个任务完成
            let handle = tokio::spawn(async move {
                execute_with_rate_limit(|| async move {
                    let _ = sem_clone.add_permits(1);
                    tcping_handler(ip, csv_clone, bar_clone, &args_clone).await;
                    Ok::<(), io::Error>(())
                }).await.unwrap();
                drop(permit);
            });
            handles.push(handle);
        }

        // 等待所有任务完成
        for handle in handles {
            let _ = handle.await;
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

pub async fn tcping(ip: IpAddr, args: &Args) -> Option<f64> {
    // 使用GLOBAL_POOL获取任务ID
    let task_id = GLOBAL_POOL.get_task_id();
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
    
    // 根据测试结果更新数据和进度条
    if recv == 0 {
        // 连接失败，获取当前可用IP数量并更新进度条
        let now_able = {
            let csv_guard = csv.lock().unwrap();
            csv_guard.len()
        };
        
        // 更新进度条
        bar.grow(1, now_able.to_string());
        return;
    }
    
    // 连接成功，创建测试数据
    let ping_times = common::get_ping_times(args);
    let data = PingData::new(ip, ping_times, recv, avg_delay_ms);
    
    // 应用筛选条件并更新进度条
    let now_able = if common::should_keep_result(&data, args) {
        // 符合条件，添加到结果集
        let mut csv_guard = csv.lock().unwrap();
        csv_guard.push(data);
        let count = csv_guard.len();
        count
    } else {
        // 不符合条件，获取当前数量
        let csv_guard = csv.lock().unwrap();
        csv_guard.len()
    };
    
    // 更新进度条
    bar.grow(1, now_able.to_string());
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