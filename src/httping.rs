use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use std::io;
use url::Url;
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
    colo_filters: Vec<String>,
    urlist: Vec<String>,
    success_count: Arc<AtomicUsize>,
    timeout_flag: Arc<AtomicBool>,
}

impl Ping {
    pub async fn new(args: &Args, timeout_flag: Arc<AtomicBool>) -> io::Result<Self> {
        // 优先使用-hu参数指定的URL列表
        let urlist = if !args.httping_urls.is_empty() {
            args.httping_urls.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            // 如果没有-hu参数，则使用-url或-urlist
            common::get_url_list(&args.url, &args.urlist).await
        };
        
        if urlist.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "警告：URL列表为空"));
        }
        
        // 解析 colo 过滤条件
        let colo_filters = if !args.httping_cf_colo.is_empty() {
            common::parse_colo_filters(&args.httping_cf_colo)
        } else {
            Vec::new()
        };
        
        // 初始化测试环境
        let (ip_buffer, csv, bar, max_loss_rate) = common::init_ping_test(args);
        
        Ok(Ping {
            ip_buffer,
            csv,
            bar,
            max_loss_rate,
            args: args.clone(),
            colo_filters,
            urlist,
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
            "Httping",
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
        let colo_filters = self.colo_filters.clone();
        let urlist = self.urlist.clone();
        let success_count = Arc::clone(&self.success_count);
        let timeout_flag = Arc::clone(&self.timeout_flag);

        // 使用FuturesUnordered来动态管理任务
        let mut tasks = FuturesUnordered::new();
        
        // 获取当前线程池的并发能力
        let initial_tasks = GLOBAL_POOL.get_concurrency_level();
        let mut url_index = 0;

        let add_task = |ip: IpAddr, url_index: &mut usize, tasks: &mut FuturesUnordered<_>| {
            // 选择URL (轮询)
            let url = urlist[*url_index % urlist.len()].clone();
            *url_index += 1;

            let csv_clone = Arc::clone(&csv);
            let bar_clone = Arc::clone(&bar);
            let args_clone = args.clone();
            let colo_filters_clone = colo_filters.clone();
            let success_count_clone = Arc::clone(&success_count);

            tasks.push(tokio::spawn(async move {
                execute_with_rate_limit(|| async move {
                    httping_handler(ip, csv_clone, bar_clone, &args_clone, colo_filters_clone, &url, success_count_clone).await;
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
                add_task(ip, &mut url_index, &mut tasks);
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
                add_task(ip, &mut url_index, &mut tasks);
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

// HTTP 测速处理函数
async fn httping_handler(
    ip: IpAddr, 
    csv: Arc<Mutex<PingDelaySet>>, 
    bar: Arc<Bar>, 
    args: &Args,
    colo_filters: Vec<String>,
    url: &str,
    success_count: Arc<AtomicUsize>,
) {
    // 执行 HTTP 连接测试
    let result = httping(ip, args, &colo_filters, url).await;

    if result.is_none() {
        // 连接失败，更新进度条（使用成功连接数）
        let current_success = success_count.load(Ordering::Relaxed);
        bar.grow(1, current_success.to_string());
        return;
    }
    
    // 连接成功，增加成功计数
    let new_success_count = success_count.fetch_add(1, Ordering::Relaxed) + 1;

    // 解包测试结果
    let (recv, avg_delay, data_center) = result.unwrap();
    
    // 创建测试数据
    let ping_times = common::get_ping_times(args);
    let mut data = PingData::new(ip, ping_times, recv, avg_delay);
    data.data_center = data_center;
    
    // 应用筛选条件（但不影响进度条计数）
    if common::should_keep_result(&data, args) {
        let mut csv_guard = csv.lock().unwrap();
        csv_guard.push(data);
    }
    
    // 更新进度条（使用成功连接数）
    bar.grow(1, new_success_count.to_string());
}

// HTTP 测速函数
async fn httping(
    ip: IpAddr, 
    args: &Args,
    colo_filters: &[String],
    url: &str
) -> Option<(u16, f32, String)> {
    
    // 开始任务
    GLOBAL_POOL.start_task();
    
    // 创建CPU计时器（仅测量本地CPU计算）
    let mut cpu_timer = GLOBAL_POOL.start_cpu_timer();
    
    // 解析URL（CPU计算部分）
    let url_parts = match Url::parse(url) {
        Ok(parts) => parts,
        Err(_) => {
            // 结束任务
            GLOBAL_POOL.end_task();
            return None;
        }
    };
    
    let host = match url_parts.host_str() {
        Some(host) => host,
        None => {
            // 结束任务
            GLOBAL_POOL.end_task();
            return None;
        }
    };
    
    let path = url_parts.path();
    let port = common::get_tcp_port(args);
    let is_https = url_parts.scheme() == "https";
    
    // URL解析完成，暂停CPU计时
    cpu_timer.pause();

    // 进行多次测速（并发执行）
    let ping_times = common::get_ping_times(args);
    let mut tasks = FuturesUnordered::new();

    for _ in 0..ping_times {
        // 恢复CPU计时（客户端构建是CPU计算）
        cpu_timer.resume();
        let client = match common::build_reqwest_client_with_host(ip, port, host, args.max_delay.as_millis().try_into().unwrap()).await {
            Some(client) => client,
            None => continue,
        };
        // 客户端构建完成，暂停CPU计时
        cpu_timer.pause();
        
        let host = host.to_string();
        let path = path.to_string();
        let is_https = is_https;
        let port = port;
    
        tasks.push(tokio::spawn({
            async move {
                let start_time = Instant::now();
                
                let result = async {
                    // 直接等待请求完成
                    common::send_head_request(&client, is_https, &host, port, &path).await
                }.await;
        
                (result, start_time)
            }
        }));
    }

    // 所有请求发送完成，恢复CPU计时（准备处理结果）
    cpu_timer.resume();

    // 处理并发任务结果
    let mut success = 0;
    let mut total_delay_ms = 0.0;
    let mut data_center = String::new();
    let mut first_request_success = false;

    while let Some(result) = tasks.next().await {
        if let Ok((Some(response), start_time)) = result {
            // 获取状态码
            let status_code = response.status().as_u16();
            
            if !common::is_valid_status_code(status_code, args) {
                continue;
            }
            
            if !first_request_success {
                first_request_success = true;
                if let Some(cf_ray) = response.headers().get("cf-ray") {
                    if let Ok(cf_ray_str) = cf_ray.to_str() {
                        data_center = common::extract_colo(cf_ray_str);
                        
                        if !args.httping_cf_colo.is_empty() {
                            if !data_center.is_empty() && !colo_filters.is_empty() {
                                let dc_upper = data_center.to_uppercase();
                                if !colo_filters.iter().any(|filter| dc_upper == *filter) {
                                    GLOBAL_POOL.end_task();
                                    return None;
                                }
                            }
                        }
                    }
                }
            }
            
            success += 1;
            total_delay_ms += start_time.elapsed().as_secs_f32() * 1000.0;
        }
    }

    // 所有结果处理完成，结束CPU计时
    cpu_timer.finish();

    // 结束任务
    GLOBAL_POOL.end_task();

    // 返回结果
    if success > 0 {
        // 使用 common 模块中的函数计算延迟
        let avg_delay_ms = common::calculate_precise_delay(total_delay_ms, success);
        Some((success, avg_delay_ms, data_center))
    } else {
        None
    }
}