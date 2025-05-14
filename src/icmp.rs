use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::io;
use futures::stream::{FuturesUnordered, StreamExt};
use std::sync::atomic::{AtomicUsize, AtomicBool, Ordering};
use surge_ping::{Client, Config, PingIdentifier, PingSequence, ICMP};
use rand::random;
use crate::pool::{execute_with_rate_limit, global_pool};

use crate::progress::Bar;
use crate::args::Args;
use crate::common::{self, PingData, PingDelaySet};
use crate::ip::IpBuffer;

pub struct Ping {
    ip_buffer: Arc<Mutex<IpBuffer>>,
    csv: Arc<Mutex<PingDelaySet>>,
    bar: Arc<Bar>,
    args: Arc<Args>,
    success_count: Arc<AtomicUsize>,
    client_v4: Arc<Client>,
    client_v6: Arc<Client>,
    timeout_flag: Arc<AtomicBool>,
}

impl Ping {
    pub async fn new(args: &Args, timeout_flag: Arc<AtomicBool>) -> io::Result<Self> {
        // 打印开始延迟测试的信息
        common::print_speed_test_info("ICMP-Ping", args);
        // 初始化测试环境
        let (ip_buffer, csv, bar) = common::init_ping_test(args);
        let client_v4 = Arc::new(Client::new(&Config::default())?);
        let client_v6 = Arc::new(Client::new(&Config::builder().kind(ICMP::V6).build())?);

        Ok(Ping {
            ip_buffer,
            csv,
            bar,
            args: Arc::new(args.clone()),
            success_count: Arc::new(AtomicUsize::new(0)),
            client_v4,
            client_v6,
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
   
        // 准备工作数据
        let ip_buffer = Arc::clone(&self.ip_buffer);
        let csv = Arc::clone(&self.csv);
        let bar = Arc::clone(&self.bar);
        let args = Arc::clone(&self.args);
        let success_count = Arc::clone(&self.success_count);
        let timeout_flag = Arc::clone(&self.timeout_flag);

        // 使用FuturesUnordered来动态管理任务
        let mut tasks = FuturesUnordered::new();
        
        // 获取当前线程池的并发能力
        let initial_tasks = global_pool().get_concurrency_level();

        let client_v4 = Arc::clone(&self.client_v4);
        let client_v6 = Arc::clone(&self.client_v6);

        let add_task = |ip: IpAddr, tasks: &mut FuturesUnordered<_>| {
            let csv_clone = Arc::clone(&csv);
            let bar_clone = Arc::clone(&bar);
            let args_clone = Arc::clone(&args);
            let success_count_clone = Arc::clone(&success_count);
            let client_v4_clone = Arc::clone(&client_v4);
            let client_v6_clone = Arc::clone(&client_v6);

            tasks.push(tokio::spawn(async move {
                icmp_handler(
                    ip, 
                    csv_clone, 
                    bar_clone, 
                    &args_clone, 
                    success_count_clone,
                    client_v4_clone,
                    client_v6_clone
                ).await;
                Ok::<(), io::Error>(())
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
            
            // 检查是否达到目标成功数量
            if let Some(target_num) = args.target_num {
                if success_count.load(Ordering::Relaxed) >= target_num as usize {
                    break;
                }
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

        // 收集所有测试结果
        let mut results = self.csv.lock().unwrap().clone();
        
        // 使用common模块的排序函数
        common::sort_results(&mut results);

        Ok(results)
    }
}

// ICMP 测速处理函数
async fn icmp_handler(
    ip: IpAddr, 
    csv: Arc<Mutex<PingDelaySet>>, 
    bar: Arc<Bar>, 
    args: &Arc<Args>,
    success_count: Arc<AtomicUsize>,
    client_v4: Arc<Client>,
    client_v6: Arc<Client>,
) {
    let ping_times = args.ping_times;
    
    // 对单个IP进行多次测试
    let mut recv = 0;
    let mut total_delay_ms = 0.0;
    
    // 根据IP类型选择客户端
    let client = match ip {
        IpAddr::V4(_) => client_v4,
        IpAddr::V6(_) => client_v6,
    };
    
    // 串行执行同一 IP 的多次测试
    for _ in 0..ping_times {
        // 执行单次测试，使用信号量控制速率
        if let Ok(Some(delay)) = execute_with_rate_limit(|| async {
            Ok::<Option<f32>, io::Error>(icmp_ping(ip, args, &client).await)
        }).await {
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

// ICMP ping函数
async fn icmp_ping(ip: IpAddr, args: &Arc<Args>, client: &Arc<Client>) -> Option<f32> {
    let payload = [0; 56];
    let identifier = PingIdentifier(random::<u16>());
    let mut rtt = None;

    let mut pinger = client.pinger(ip, identifier).await;
    pinger.timeout(args.max_delay);

    match pinger.ping(PingSequence(0), &payload).await {
        Ok((packet, dur)) => {
            rtt = Some(dur.as_secs_f32() * 1000.0);
            drop(packet);
        },
        Err(_) => {}
    }
    rtt
}
