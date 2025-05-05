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
    max_loss_rate: f32,
    args: Args,
    success_count: Arc<AtomicUsize>,
    client_v4: Arc<Client>,
    client_v6: Arc<Client>,
    timeout_flag: Arc<AtomicBool>,
}

impl Ping {
    pub async fn new(args: &Args, timeout_flag: Arc<AtomicBool>) -> io::Result<Self> {
        let (ip_buffer, csv, bar, max_loss_rate) = common::init_ping_test(args);
        let client_v4 = Arc::new(Client::new(&Config::default())?);
        let client_v6 = Arc::new(Client::new(&Config::builder().kind(ICMP::V6).build())?);

        Ok(Ping {
            ip_buffer,
            csv,
            bar,
            max_loss_rate,
            args: args.clone(),
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

        // 打印开始延迟测试的信息
        common::print_speed_test_info(
            "ICMP-Ping",
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
        let initial_tasks = global_pool().get_concurrency_level();

        let client_v4 = Arc::clone(&self.client_v4);
        let client_v6 = Arc::clone(&self.client_v6);

        let add_task = |ip: IpAddr, tasks: &mut FuturesUnordered<_>| {
            let csv_clone = Arc::clone(&csv);
            let bar_clone = Arc::clone(&bar);
            let args_clone = args.clone();
            let success_count_clone = Arc::clone(&success_count);
            let client_v4_clone = Arc::clone(&client_v4);
            let client_v6_clone = Arc::clone(&client_v6);

            tasks.push(tokio::spawn(async move {
                execute_with_rate_limit(|| async move {
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
        common::sort_ping_results(&mut results);
        
        // 去除重复IP，只保留首个出现的IP
        let mut unique_results = Vec::new();
        let mut seen_ips = std::collections::HashSet::new();

        for data in results {
            if seen_ips.insert(data.ip) {
                unique_results.push(data);
            }
        }

        Ok(unique_results)
    }
}

// TCP 测速处理函数
async fn icmp_handler(
    ip: IpAddr, 
    csv: Arc<Mutex<PingDelaySet>>, 
    bar: Arc<Bar>, 
    args: &Args,
    success_count: Arc<AtomicUsize>,
    client_v4: Arc<Client>,
    client_v6: Arc<Client>,
) {
    let ping_times = common::get_ping_times(args);
    let mut tasks = FuturesUnordered::new();

    // 使用FuturesUnordered来动态管理并发测试
    for _ in 0..ping_times {
        let args_clone = args.clone();
        let client_v4_clone = Arc::clone(&client_v4);
        let client_v6_clone = Arc::clone(&client_v6);
        
        tasks.push(tokio::spawn(async move {
            icmp_ping(
                ip, 
                &args_clone, 
                client_v4_clone,
                client_v6_clone
            ).await
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

// ICMP ping函数
async fn icmp_ping(ip: IpAddr, args: &Args, client_v4: Arc<Client>, client_v6: Arc<Client>) -> Option<f32> {
    let client = match ip {
        IpAddr::V4(_) => client_v4,
        IpAddr::V6(_) => client_v6,
    };
    let mut cpu_timer = global_pool().start_cpu_timer();

    let client = Arc::clone(&client);
    let payload = [0; 56];
    let identifier = PingIdentifier(random::<u16>());
    let mut rtt = None;

    let mut pinger = client.pinger(ip, identifier).await;
    pinger.timeout(args.max_delay);

    // 暂停 CPU 计时（网络等待阶段）
    cpu_timer.pause();
    match pinger.ping(PingSequence(0), &payload).await {
        Ok((packet, dur)) => {
            rtt = Some(dur.as_secs_f32() * 1000.0);
            drop(packet);
        },
        Err(_) => {}
    }
    // 恢复 CPU 计时（结果处理阶段）
    cpu_timer.resume();

    cpu_timer.finish();
    rtt
}