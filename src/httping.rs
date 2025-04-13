use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use url::Url;

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
    colo_filters: Vec<String>,
    urlist: Vec<String>,
}

impl Ping {
    pub async fn new(args: &Args) -> io::Result<Self> {
        let (ip_buffer, csv, bar, max_loss_rate) = common::init_ping_test(args);
        
        // 解析 colo 过滤条件，使用 common 模块中的函数
        let colo_filters = if !args.httping_cf_colo.is_empty() {
            common::parse_colo_filters(&args.httping_cf_colo)
        } else {
            Vec::new()
        };
        
        // 使用common模块获取URL列表
        let urlist = common::get_url_list(&args.url, &args.urlist).await;
        
        if urlist.is_empty() {
            println!("警告：URL列表为空，将使用默认URL");
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "URL列表为空"));
        }
        
        Ok(Ping {
            ip_buffer,
            csv,
            bar,
            max_loss_rate,
            args: args.clone(),
            colo_filters,
            urlist,
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
            "Httping",
            common::get_tcp_port(&self.args),
            common::get_min_delay(&self.args),
            common::get_max_delay(&self.args),
            self.max_loss_rate
        );

        // 3. 创建线程安全集合和任务句柄向量
        let mut handles = Vec::new();
        let mut url_index = 0;

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
            let colo_filters = self.colo_filters.clone();
            
            // 根据索引选择URL（循环使用）
            let url = self.urlist[url_index % self.urlist.len()].clone();
            url_index += 1;

            // 创建异步任务
            let handle = tokio::spawn(async move {
                // 申请全局并发控制许可
                execute_with_rate_limit(|| async {
                    httping_handler(ip, csv, bar, &args, colo_filters, &url).await;
                    Ok::<(), io::Error>(())
                }).await.unwrap();
            });

            handles.push(handle);
        }

        // 5. 等待所有异步任务完成
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

// HTTP 测速处理函数
async fn httping_handler(
    ip: IpAddr, 
    csv: Arc<Mutex<PingDelaySet>>, 
    bar: Arc<Bar>, 
    args: &Args,
    colo_filters: Vec<String>,
    url: &str
) {
    // 执行 HTTP 连接测试
    let result = httping(ip, args, &colo_filters, url).await;
    
    // 如果测试失败，直接更新进度条并返回
    if result.is_none() {
        // 获取当前可用IP数量并更新进度条
        let now_able = {
            let csv_guard = csv.lock().unwrap();
            csv_guard.len()
        };
        
        // 更新进度条
        bar.grow(1, now_able.to_string());
        return;
    }
    
    // 解包测试结果
    let (recv, avg_delay, data_center) = result.unwrap();
    
    // 连接成功，创建测试数据
    let ping_times = common::get_ping_times(args);
    let mut data = PingData::new(ip, ping_times, recv, avg_delay);
    data.data_center = data_center;
    
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

// HTTP 测速函数
async fn httping(
    ip: IpAddr, 
    args: &Args,
    colo_filters: &[String],
    url: &str
) -> Option<(usize, f64, String)> {
    // 1. 生成唯一任务标识并记录任务开始
    let task_id = TASK_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
    GLOBAL_POOL.start_task(task_id);
    
    // 解析URL
    let url_parts = match Url::parse(url) {
        Ok(parts) => parts,
        Err(_) => {
            GLOBAL_POOL.end_task(task_id);
            return None;
        }
    };
    
    let host = match url_parts.host_str() {
        Some(host) => host,
        None => {
            GLOBAL_POOL.end_task(task_id);
            return None;
        }
    };
    
    let path = url_parts.path();
    let port = common::get_tcp_port(args);
    let is_https = url_parts.scheme() == "https";
    
    // 2. 进行多次测速
    let ping_times = common::get_ping_times(args);
    let mut success = 0;
    let mut total_delay_ms = 0.0; // 使用f64类型表示毫秒
    let mut data_center = String::new();
    let mut first_request_success = false; // 标记是否是第一个成功的请求

    for _ in 0..ping_times {
        // 构建新的 reqwest 客户端
        let client = match common::build_reqwest_client_with_host(ip, port, host, args.max_delay.as_millis() as u64).await {
            Some(client) => client,
            None => continue,
        };
        
        let start_time = Instant::now();
        
        // 使用timeout监听请求，采用内部心跳方式
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        
        let result = tokio::time::timeout(args.max_delay, async {
            // 创建请求future
            let request_future = common::send_head_request(&client, is_https, host, port, path);
            
            // 使用select同时处理请求和心跳
            tokio::pin!(request_future);
            
            loop {
                tokio::select! {
                    request_result = &mut request_future => {
                        return request_result;
                    },
                    _ = interval.tick() => {
                        // 记录进度
                        GLOBAL_POOL.record_progress(task_id);
                    }
                }
            }
        }).await;
        
        match result {
            Ok(Some(response)) => {
                // 获取状态码
                let status_code = response.status().as_u16();
                
                // 验证状态码 - 每次请求都验证
                if !common::is_valid_status_code(status_code, args) {
                    continue; // 状态码不匹配，当前请求算作失败
                }
                
                // 如果这是第一个成功的请求，提取数据中心信息
                if !first_request_success {
                    first_request_success = true;
                    
                    // 提取数据中心信息
                    if let Some(cf_ray) = response.headers().get("cf-ray") {
                        if let Ok(cf_ray_str) = cf_ray.to_str() {
                            data_center = common::extract_colo(cf_ray_str);
                            
                            // 只有当指定了 httping_cf_colo 参数时才进行数据中心匹配检查
                            if !args.httping_cf_colo.is_empty() {
                                // 检查数据中心是否匹配
                                if !data_center.is_empty() && !colo_filters.is_empty() {
                                    let dc_upper = data_center.to_uppercase();
                                    if !colo_filters.iter().any(|filter| dc_upper == *filter) {
                                        GLOBAL_POOL.end_task(task_id);
                                        return None; // 数据中心不匹配，直接返回失败
                                    }
                                }
                            }
                        }
                    }
                }
                
                // 请求成功
                success += 1;
                total_delay_ms += start_time.elapsed().as_secs_f64() * 1000.0;
            },
            _ => {
                // 请求失败或超时
            }
        }
    }

    // 3. 结束任务
    GLOBAL_POOL.end_task(task_id);

    // 4. 返回结果
    if success > 0 {
        // 使用 common 模块中的函数计算延迟
        let avg_delay_ms = common::calculate_precise_delay(total_delay_ms, success);
        Some((success, avg_delay_ms, data_center))
    } else {
        None
    }
}
