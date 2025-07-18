use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use std::io;
use url::Url;
use futures::stream::{FuturesUnordered, StreamExt};
use std::sync::atomic::{AtomicUsize, AtomicBool, Ordering};
use std::future::Future;
use std::pin::Pin;

use crate::progress::Bar;
use crate::args::Args;
use crate::pool::execute_with_rate_limit;
use crate::common::{self, PingData, PingDelaySet, HandlerFactory, BaseHandlerFactory};

pub struct Ping {
    base: common::BasePing,
    colo_filters: Vec<String>,
    urlist: Vec<String>,
    use_https: bool,
}

pub struct HttpingHandlerFactory {
    base: BaseHandlerFactory,
    colo_filters: Arc<Vec<String>>,
    urls: Arc<Vec<String>>,
    url_index: Arc<AtomicUsize>,
    use_https: bool,
}

impl HandlerFactory for HttpingHandlerFactory {
    fn create_handler(&self, addr: SocketAddr) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let (csv, bar, args, success_count) = self.base.clone_shared_state();
        let colo_filters = Arc::clone(&self.colo_filters);
        let urls = Arc::clone(&self.urls);
        let url_index = Arc::clone(&self.url_index);
        let use_https = self.use_https;
        
        Box::pin(async move {
            // 根据模式选择URL
            let url = if use_https {
                // HTTPS模式：从URL列表中选择（轮询）
                let current_index = url_index.fetch_add(1, Ordering::Relaxed) % urls.len();
                Arc::new(urls[current_index].clone())
            } else {
                // 非HTTPS模式：直接使用IP构建URL
                let mut host_str = addr.ip().to_string();
                if let IpAddr::V6(_) = addr.ip() {
                    host_str = format!("[{}]", addr.ip());
                }
                Arc::new(Ping::build_trace_url("http", &host_str))
            };

            httping_handler(addr, csv, bar, &args, &colo_filters, &url, success_count).await;
        })
    }
}

impl Ping {
    fn build_trace_url(scheme: &str, host: &str) -> String {
        format!("{}://{}/cdn-cgi/trace", scheme, host)
    }

    pub async fn new(args: &Args, timeout_flag: Arc<AtomicBool>) -> io::Result<Self> {
        // 判断是否使用-hu参数（无论是否传值）
        let use_https = !args.httping_urls.is_empty() || args.httping_urls_flag;
        
        let urlist = if use_https {
            let url_to_trace = |url: &str| -> String {
                if let Ok(parsed) = Url::parse(url) {
                    if let Some(host) = parsed.host_str() {
                        return Self::build_trace_url("https", host);
                    }
                }
                Self::build_trace_url("https", url)
            };
            
            if !args.httping_urls.is_empty() {
                // -hu参数有值，使用指定的URL列表
                args.httping_urls.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .map(|url| url_to_trace(&url))
                    .collect()
            } else {
                // -hu参数无值，从-url或-urlist获取域名列表
                let url_list = common::get_url_list(&args.url, &args.urlist).await;
                // 只提取域名部分
                url_list.iter()
                    .map(|url| url_to_trace(url))
                    .collect()
            }
        } else {
            // 不使用HTTPS模式，无需获取URL列表
            Vec::new()
        };
        
        // 如果使用HTTPS模式但URL列表为空，返回错误
        if use_https && urlist.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "警告：URL列表为空"));
        }
        
        // 解析 colo 过滤条件
        let colo_filters = if !args.httping_cf_colo.is_empty() {
            common::parse_colo_filters(&args.httping_cf_colo)
        } else {
            Vec::new()
        };

        // 打印开始延迟测试的信息
        common::print_speed_test_info("Httping", args);
        
        // 初始化测试环境
        let base = common::create_base_ping(args, timeout_flag);

        Ok(Ping {
            base,
            colo_filters,
            urlist,
            use_https,
        })
    }

    fn make_handler_factory(
        &self,
    ) -> Arc<dyn HandlerFactory> {
        Arc::new(HttpingHandlerFactory {
            base: BaseHandlerFactory {
                csv: Arc::clone(&self.base.handler_factory.csv),
                bar: Arc::clone(&self.base.handler_factory.bar),
                args: Arc::clone(&self.base.handler_factory.args),
                success_count: Arc::clone(&self.base.handler_factory.success_count),
            },
            colo_filters: Arc::new(self.colo_filters.clone()),
            urls: Arc::new(self.urlist.clone()),
            url_index: Arc::new(AtomicUsize::new(0)),
            use_https: self.use_https,
        })
    }

    pub async fn run(self) -> Result<PingDelaySet, io::Error> {
        // 检查HTTPS模式下URL列表是否为空
        if self.use_https && self.urlist.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "警告：URL列表为空"));
        }

        let handler_factory = self.make_handler_factory();
        common::run_ping_test(&self.base, handler_factory).await
    }
}

// HTTP 测速处理函数
async fn httping_handler(
    addr: SocketAddr,
    csv: Arc<Mutex<PingDelaySet>>, 
    bar: Arc<Bar>, 
    args: &Arc<Args>,
    colo_filters: &Arc<Vec<String>>,
    url: &Arc<String>,
    success_count: Arc<AtomicUsize>,
) {
    // 执行 HTTP 连接测试
    let result = httping(addr, args, &colo_filters, url).await;

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
    let ping_times = args.ping_times;
    let mut data = PingData::new(addr, ping_times, recv, avg_delay);
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
    addr: SocketAddr,
    args: &Arc<Args>,
    colo_filters: &Arc<Vec<String>>,
    url: &Arc<String>,
) -> Option<(u16, f32, String)> {
    
    // 解析URL获取主机名
    let host = {
        // 解析URL
        let url_parts = Url::parse(url.as_str()).ok()?;
        
        match url_parts.host_str() {
            Some(host) => host.to_string(),
            None => {
                return None;
            }
        }
    };

    // 创建客户端
    let client = match common::build_reqwest_client(addr, &host, 2000).await {
        Some(client) => client,
        None => return None,
    };

    // 进行多次测速（并发执行）
    let ping_times = args.ping_times;
    let mut tasks = FuturesUnordered::new();

    for _ in 0..ping_times {
        let client = client.clone(); // 复用客户端
        let url_clone = Arc::clone(url);
    
        // 每个HTTP请求都受到信号量控制
        tasks.push(tokio::spawn(async move {
            execute_with_rate_limit(|| async move {
                let start_time = Instant::now();
                
                let result = client.head(url_clone.as_str())
                    .header("Connection", "close")
                    .send()
                    .await
                    .ok();
                
                Ok::<(Option<reqwest::Response>, Instant), io::Error>((result, start_time))
            }).await
        }));
    }

    // 处理并发任务结果
    let mut success = 0;
    let mut total_delay_ms = 0.0;
    let mut data_center = String::new();
    let mut first_request_success = false;

    while let Some(result) = tasks.next().await {
        if let Ok(Ok((Some(response), start_time))) = result {
            // 使用 common::extract_data_center 提取数据中心信息
            if let Some(dc) = common::extract_data_center(&response) {
                if !first_request_success {
                    first_request_success = true;
                    data_center = dc;
                    
                    if !args.httping_cf_colo.is_empty() && !data_center.is_empty() && 
                    !colo_filters.is_empty() && !common::is_colo_matched(&data_center, &colo_filters) {
                     return None;
                 }
                }
                
                success += 1;
                total_delay_ms += start_time.elapsed().as_secs_f32() * 1000.0;
            }
        }
    }

    // 返回结果
    if success > 0 {
        // 使用 common 模块中的函数计算延迟
        let avg_delay_ms = common::calculate_precise_delay(total_delay_ms, success);
        Some((success, avg_delay_ms, data_center))
    } else {
        None
    }
}