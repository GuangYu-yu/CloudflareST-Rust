use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

use crate::hyper::{parse_url_to_uri, build_hyper_client, send_head_request};
use crate::args::Args;
use crate::common::{self, HandlerFactory, PingData, BasePing, Ping as CommonPing};
use crate::pool::execute_with_rate_limit;
use crate::warning_println;

#[derive(Clone)]
pub struct HttpingFactoryData {
    colo_filters: Arc<Vec<String>>,
    urlist: Arc<Vec<Arc<String>>>,
    use_https: bool,
    interface: Option<String>,
    allowed_codes: Option<Arc<Vec<u16>>>,
}

// 实现 PingMode Trait
impl common::PingMode for HttpingFactoryData {
    fn create_handler_factory(&self, base: &BasePing) -> Arc<dyn HandlerFactory> {
        Arc::new(HttpingHandlerFactory {
            base: Arc::new(base.clone()),
            colo_filters: Arc::clone(&self.colo_filters),
            urls: Arc::clone(&self.urlist),
            url_index: Arc::new(AtomicUsize::new(0)),
            use_https: self.use_https,
            interface: self.interface.clone(),
            allowed_codes: self.allowed_codes.clone(),
        })
    }
    
    fn clone_box(&self) -> Box<dyn common::PingMode> {
        Box::new(self.clone())
    }
}

pub struct HttpingHandlerFactory {
    base: Arc<BasePing>,
    colo_filters: Arc<Vec<String>>,
    urls: Arc<Vec<Arc<String>>>,
    url_index: Arc<AtomicUsize>,
    use_https: bool,
    interface: Option<String>,
    allowed_codes: Option<Arc<Vec<u16>>>,
}

impl HandlerFactory for HttpingHandlerFactory {
    fn create_handler(
        &self,
        addr: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = Option<PingData>> + Send>> {
        // 克隆所需的引用
        let base = self.base.clone();
        let args = base.args.clone();
        let colo_filters = self.colo_filters.clone();
        let urls = self.urls.clone();
        let url_index = self.url_index.clone();
        let use_https = self.use_https;
        let interface = self.interface.clone();
        let allowed_codes = self.allowed_codes.clone();

        Box::pin(async move {
            // 统一在这里构建 URL
            const DEFAULT_TRACE_PATH: &str = "/cdn-cgi/trace";
            let (scheme, host_header, path) = if use_https {
                // HTTPS: 轮询获取域名
                let idx = url_index.fetch_add(1, Ordering::Relaxed) % urls.len();
                let raw = &urls[idx];
                
                let (host, path) = parse_url_to_uri(raw)
                    .map(|(uri, host)| { 
                        let path = if !uri.path().is_empty() {uri.path()} else {DEFAULT_TRACE_PATH};
                        (host, path.to_string()) 
                    }) 
                    .unwrap_or_else(|| { 
                        (raw.to_string(), DEFAULT_TRACE_PATH.to_string()) 
                    });
                ("https", host, path)
            } else {
                // HTTP: 使用 IP 地址
                ("http", addr.to_string(), DEFAULT_TRACE_PATH.to_string())
            };

            let ping_times = args.ping_times;
            let should_continue = Arc::new(AtomicBool::new(true));

            // 创建客户端
            let (uri, host_header) = match parse_url_to_uri(&format!("{scheme}://{host_header}{path}")) {
                Some(result) => result,
                None => return None,
            };

            // 获取并使用绑定的网络接口信息
            let interface_ref = interface.as_deref();
            let client = match build_hyper_client(
                addr,
                interface_ref,
                args.interface_ips.as_ref(),
                1800,
            ) {
                Some(client) => Arc::new(client),
                None => return None,
            };

            // 使用通用的ping循环函数
            let (avg_delay, final_data_center) = {
                let local_data_center = Arc::new(std::sync::Mutex::new(None));
                let host_header = host_header.clone();
                
                let avg_delay = common::run_ping_loop(ping_times, 200, {
                    let client = client.clone();
                    let colo_filters = colo_filters.clone();
                    let allowed_codes = allowed_codes.clone();
                    let should_continue = should_continue.clone();
                    let args = args.clone();
                    let uri = uri.clone();
                    let local_data_center = local_data_center.clone();
                    
                    move || {
                        let client = client.clone();
                        let colo_filters = colo_filters.clone();
                        let allowed_codes = allowed_codes.clone();
                        let should_continue = should_continue.clone();
                        let args = args.clone();
                        let uri = uri.clone();
                        let host_header = host_header.clone();
                        let local_data_center = local_data_center.clone();
                        
                        Box::pin(async move {
                            if !should_continue.load(Ordering::Relaxed) {
                                return None;
                            }

                            match execute_with_rate_limit(|| async move {
                                let start_time = Instant::now();

                                // 发起 HEAD 请求
                                let delay_result = {
                                    let result = {
                                        // 发送 HEAD 请求
                                        send_head_request(&client, &host_header, uri.clone(), 1200, false)
                                            .await
                                            .ok()
                                    };

                                    // 只有当所有条件都满足时才计算延迟
                                    result.and_then(|response| {
                                        // 检查状态码
                                        let status_valid = if let Some(ref codes) = allowed_codes {
                                            codes.contains(&response.status().as_u16())
                                        } else {
                                            true // 没有状态码限制时视为有效
                                        };

                                        if status_valid {
                                            // 提取数据中心信息（Colo）并计算请求延迟
                                            common::extract_data_center(&response).map(|dc| {
                                                let delay = start_time.elapsed().as_secs_f32() * 1000.0;
                                                (delay, dc)
                                            })
                                        } else {
                                            None
                                        }
                                    })
                                };

                                Ok::<Option<(f32, String)>, io::Error>(delay_result)
                            })
                            .await
                            {
                                Ok(Some((delay, dc))) => {
                                    // 首次成功响应时进行数据中心检查
                                    let mut dc_guard = local_data_center.lock().unwrap();
                                    if dc_guard.is_none() {
                                        // 检查数据中心（Colo）是否符合过滤要求
                                        if !args.httping_cf_colo.is_empty()
                                            && !common::is_colo_matched(&dc, &colo_filters)
                                        {
                                            should_continue.store(false, Ordering::Relaxed);
                                            return None;
                                        }
                                        
                                        *dc_guard = Some(dc);
                                    }

                                    Some(delay)
                                }
                                _ => None,
                            }
                        })
                    }
                }).await;
                
                let final_data_center = local_data_center.lock().unwrap().take();
                (avg_delay, final_data_center)
            };

            // 如果因 Colo 不匹配而终止，则不返回结果
            if !should_continue.load(Ordering::Relaxed) {
                return None;
            }

            if let Some(avg_delay_ms) = avg_delay {
                // 构造 PingData 结构体
                let mut data = PingData::new(addr, ping_times, ping_times, avg_delay_ms);
                if let Some(dc) = final_data_center {
                    data.data_center = dc;
                }

                // 返回 Ping 结果
                Some(data)
            } else {
                // 没有成功连接或响应，返回 None
                None
            }
        })
    }
}

pub fn new(args: &Args, timeout_flag: Arc<AtomicBool>) -> io::Result<CommonPing> {
    // 判断是否使用-hu参数（无论是否传值）
    let use_https = args.httping_urls.is_some();

    let urlist = if use_https {
        let raw_urls = match args.httping_urls {
            Some(ref urls) if !urls.is_empty() => {
                // -hu参数有值，使用指定的URL列表
                urls
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect::<Vec<_>>()
            },

            _ => {
                // -hu 未指定 URL，从 -url 或 -urlist 获取域名列表
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current()
                        .block_on(common::get_url_list(&args.url, &args.urlist))
                })
            },
        };

        raw_urls.into_iter().map(Arc::new).collect()
    } else {
        Vec::new()
    };

    // 如果使用HTTPS模式但URL列表为空，输出警告
    if use_https && urlist.is_empty() {
        warning_println(format_args!("URL列表为空"));
        std::process::exit(1);
    }

    // 解析 Colo 过滤条件
    let colo_filters = if !args.httping_cf_colo.is_empty() {
        common::parse_colo_filters(&args.httping_cf_colo)
    } else {
        Vec::new()
    };

    // 预解析状态码列表
    let allowed_codes = if !args.httping_code.is_empty() {
        Some(Arc::new(
            args.httping_code
                .split(',')
                .filter_map(|s| s.trim().parse::<u16>().ok())
                .collect::<Vec<u16>>()
        ))
    } else {
        None
    };

    // 打印开始延迟测试的信息
    let mode_name = if use_https { "HTTPSing" } else { "HTTPing" };
    common::print_speed_test_info(mode_name, args);

    // 初始化测试环境
    let base = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(common::create_base_ping(args, timeout_flag))
    });

    let factory_data = HttpingFactoryData {
        colo_filters: Arc::new(colo_filters),
        urlist: Arc::new(urlist),
        use_https,
        interface: args.interface.clone(),
        allowed_codes,
    };

    Ok(CommonPing::new(base, factory_data))
}