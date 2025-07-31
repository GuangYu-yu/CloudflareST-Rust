use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Instant;
use std::io;
use url::Url;
use std::sync::atomic::{AtomicUsize, AtomicBool, Ordering};
use std::future::Future;
use std::pin::Pin;

use crate::args::Args;
use crate::pool::execute_with_rate_limit;
use crate::common::{self, PingData, PingDelaySet, HandlerFactory};

pub struct Ping {
    base: common::BasePing,
    colo_filters: Vec<String>,
    urlist: Vec<String>,
    use_https: bool,
}

pub struct HttpingHandlerFactory {
    base: common::BasePing,
    colo_filters: Vec<String>,
    urls: Vec<String>,
    url_index: Arc<AtomicUsize>,
    use_https: bool,
}

impl HandlerFactory for HttpingHandlerFactory {
    fn create_handler(&self, addr: SocketAddr) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let (csv, bar, args, success_count) = self.base.clone_shared_state();
        let colo_filters = self.colo_filters.clone();
        let urls = self.urls.clone();
        let url_index = Arc::clone(&self.url_index);
        let use_https = self.use_https;

        Box::pin(async move {
            // 根据模式选择URL
            let url = Arc::new(if use_https {
                // HTTPS模式：从URL列表中选择（轮询）
                let current_index = url_index.fetch_add(1, Ordering::Relaxed) % urls.len();
                urls[current_index].clone()
            } else {
                // 非HTTPS模式：直接使用IP构建URL
                let mut host_str = addr.ip().to_string();
                if let IpAddr::V6(_) = addr.ip() {
                    host_str = format!("[{}]", addr.ip());
                }
                Ping::build_trace_url("http", &host_str)
            });

            let ping_times = args.ping_times;
            let mut delays = Vec::with_capacity(ping_times as usize);
            let mut data_center = None;
            let mut should_continue = true;

            // 创建客户端
            let host = match Url::parse(&url) {
                Ok(url_parts) => match url_parts.host_str() {
                    Some(host) => host.to_string(),
                    None => {
                        // 连接失败，更新进度条
                        let current_success = success_count.load(Ordering::Relaxed);
                        bar.grow(1, current_success.to_string());
                        return;
                    }
                },
                Err(_) => {
                    // 连接失败，更新进度条
                    let current_success = success_count.load(Ordering::Relaxed);
                    bar.grow(1, current_success.to_string());
                    return;
                }
            };

            let client = match common::build_reqwest_client(addr, &host, 1800).await {
                Some(client) => Arc::new(client),
                None => {
                    // 连接失败，更新进度条
                    let current_success = success_count.load(Ordering::Relaxed);
                    bar.grow(1, current_success.to_string());
                    return;
                }
            };

            // 解析允许的状态码列表
            let allowed_codes = Arc::new((!args.httping_code.is_empty()).then(|| {
                args.httping_code
                    .split(',')
                    .filter_map(|s| s.trim().parse::<u16>().ok())
                    .collect::<Vec<u16>>()
            }));

            for i in 0..ping_times {
                // 检查是否需要继续测试
                if !should_continue {
                    break;
                }

                let client = Arc::clone(&client);
                let colo_filters = &colo_filters;
                let allowed_codes = &*allowed_codes;
                let url = Arc::clone(&url);

                match execute_with_rate_limit(|| async move {
                    let start_time = Instant::now();

                    // 构造请求
                    let result = {
                        let builder = client.head(url.as_str());
                        if i == ping_times - 1 { builder.header("Connection", "close") } else { builder }
                    }.send().await.ok();

                    if let Some(response) = result {
                        // 判断状态码
                        if let Some(ref codes) = *allowed_codes {
                            let status = response.status().as_u16();
                            if !codes.contains(&status) {
                                return Ok::<Option<(f32, String)>, io::Error>(None);
                            }
                        }

                        if let Some(dc) = common::extract_data_center(&response) {
                            let delay = start_time.elapsed().as_secs_f32() * 1000.0;
                            return Ok(Some((delay, dc)));
                        }
                    }

                    Ok::<Option<(f32, String)>, io::Error>(None)
                })
                .await
                {
                    Ok(Some((delay, dc))) => {
                        // 成功时等待200ms
                        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

                        // 如果是第一次成功响应
                        if data_center.is_none() {
                            // 检查数据中心过滤条件
                            if !args.httping_cf_colo.is_empty() && 
                            !common::is_colo_matched(&dc, &colo_filters)
                            {
                                should_continue = false;
                                continue; // 跳过后续处理
                            }

                            data_center = Some(dc);
                        }

                        delays.push(delay);
                    },
                    _ => {
                        // 失败或错误情况，不做特殊处理
                    }
                }
            }

            // 如果因为数据中心不匹配而终止测试，则不记录结果
            if !should_continue {
                // 更新进度条但不记录结果
                let current_success = success_count.load(Ordering::Relaxed);
                bar.grow(1, current_success.to_string());
                return;
            }

            // 计算成功次数和平均延迟
            let recv = delays.len();
            if recv > 0 {
                let total_delay_ms: f32 = delays.iter().sum();
                let avg_delay_ms = common::calculate_precise_delay(total_delay_ms, recv as u16);

                // 增加成功计数
                let new_success_count = success_count.fetch_add(1, Ordering::Relaxed) + 1;

                // 创建测试数据
                let mut data = PingData::new(addr, ping_times, recv as u16, avg_delay_ms);
                if let Some(dc) = data_center {
                    data.data_center = dc;
                }

                // 应用筛选条件
                if common::should_keep_result(&data, &args) {
                    let mut csv_guard = csv.lock().unwrap();
                    csv_guard.push(data);
                }

                // 更新进度条（使用成功连接数）
                bar.grow(1, new_success_count.to_string());
            } else {
                // 没有成功连接，但也需要更新进度条
                let current_success = success_count.load(Ordering::Relaxed);
                bar.grow(1, current_success.to_string());
            }
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
            base: self.base.clone(),
            colo_filters: self.colo_filters.clone(),
            urls: self.urlist.clone(),
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