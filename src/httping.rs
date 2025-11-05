use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

use crate::hyper::{parse_url_to_uri, build_hyper_client, send_head_request};
use crate::args::Args;
use crate::common::{self, HandlerFactory, PingData, PingDelaySet};
use crate::pool::execute_with_rate_limit;

pub struct Ping {
    base: common::BasePing,
    colo_filters: Vec<String>,
    urlist: Vec<String>,
    use_https: bool,
}

pub struct HttpingHandlerFactory {
    base: Arc<common::BasePing>,
    colo_filters: Arc<Vec<String>>,
    urls: Arc<Vec<String>>,
    url_index: Arc<AtomicUsize>,
    use_https: bool,
    interface: Option<String>,
}

impl HandlerFactory for HttpingHandlerFactory {
    fn create_handler(
        &self,
        addr: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = Option<PingData>> + Send>> {
        // 克隆所需的 Arc 引用
        let base = self.base.clone();
        let args = base.args.clone();
        let colo_filters = self.colo_filters.clone();
        let urls = self.urls.clone();
        let url_index = self.url_index.clone();
        let use_https = self.use_https;
        let interface = self.interface.clone();

        Box::pin(async move {
            // 根据模式选择URL
            let url = if use_https {
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
            };

            let ping_times = args.ping_times;
            let mut delays = Vec::with_capacity(ping_times as usize);
            let mut data_center = None;
            let mut should_continue = true;

            // 创建客户端
            let host = match parse_url_to_uri(&url) {
                Some((_, host)) => host,
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

            // 预解析一次允许的 HTTP 状态码列表
            let allowed_codes = if !args.httping_code.is_empty() {
                Some(
                    args.httping_code
                        .split(',')
                        .filter_map(|s| s.trim().parse::<u16>().ok())
                        .collect::<Vec<u16>>()
                )
            } else {
                None
            };

            for i in 0..ping_times {
                // 检查是否需要继续测试
                if !should_continue {
                    break;
                }

                let client = client.clone();
                let colo_filters = colo_filters.clone();
                let allowed_codes = allowed_codes.clone();
                let url = url.clone();
                let host = host.clone();

                match execute_with_rate_limit(|| async move {
                    let start_time = Instant::now();

                    // 发起 HEAD 请求
                    let delay_result = {
                        let result = {
                            // 解析 URL 字符串为 hyper::Uri
                            let (uri, _) = match parse_url_to_uri(&url) {
                                Some(result) => result,
                                None => return Ok(None),
                            };

                            // 判断是否为最后一次 ping，决定是否发送 Connection: close
                            let close_connection = i == ping_times - 1;

                            // 发送 HEAD 请求，并传递连接关闭标志
                            send_head_request(&client, &host, uri, 1800, close_connection)
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
                        // 成功后等待 200ms 间隔
                        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

                        // 首次成功响应时进行数据中心检查
                        if data_center.is_none() {
                            // 检查数据中心（Colo）是否符合过滤要求
                            if !args.httping_cf_colo.is_empty()
                                && !common::is_colo_matched(&dc, &colo_filters)
                            {
                                should_continue = false;
                                continue; // 不符合过滤要求，跳过后续 ping
                            }

                            data_center = Some(dc);
                        }

                        delays.push(delay);
                    }
                    _ => {
                        // 失败或错误，不做特殊处理
                    }
                }
            }

            // 如果因 Colo 不匹配而终止，则不返回结果
            if !should_continue {
                return None;
            }

            // 计算成功次数和平均延迟
            let recv = delays.len();
            if recv > 0 {
                let total_delay_ms: f32 = delays.iter().sum();
                let avg_delay_ms = common::calculate_precise_delay(total_delay_ms, recv as u16);

                // 构造 PingData 结构体
                let mut data = PingData::new(addr, ping_times, recv as u16, avg_delay_ms);
                if let Some(dc) = data_center {
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

impl Ping {
    fn build_trace_url(scheme: &str, host: &str) -> String {
        format!("{}://{}/cdn-cgi/trace", scheme, host)
    }

    pub async fn new(args: &Args, timeout_flag: Arc<AtomicBool>) -> io::Result<Self> {
        // 判断是否使用-hu参数（无论是否传值）
        let use_https = args.httping_urls.is_some();

        let urlist = if use_https {
            let url_to_trace = |url: &str| -> String {
                if let Some((_, host)) = parse_url_to_uri(url) {
                    return Self::build_trace_url("https", &host);
                }
                Self::build_trace_url("https", url)
            };

            if let Some(ref urls) = args.httping_urls {
                if !urls.is_empty() {
                    // -hu参数有值，使用指定的URL列表
                    urls.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .map(|url| url_to_trace(&url))
                        .collect()
                } else {
                    // -hu 未指定 URL，从 -url 或 -urlist 获取域名列表
                    let url_list = common::get_url_list(&args.url, &args.urlist).await;
                    url_list.iter().map(|url| url_to_trace(url)).collect()
                }
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        // 如果使用HTTPS模式但URL列表为空，返回错误
        if use_https && urlist.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "警告：URL列表为空",
            ));
        }

        // 解析 Colo 过滤条件
        let colo_filters = if !args.httping_cf_colo.is_empty() {
            common::parse_colo_filters(&args.httping_cf_colo)
        } else {
            Vec::new()
        };

        // 打印开始延迟测试的信息
        common::print_speed_test_info("Httping", args);

        // 初始化测试环境
        let base = common::create_base_ping(args, timeout_flag).await;

        Ok(Ping {
            base,
            colo_filters,
            urlist,
            use_https,
        })
    }

    fn make_handler_factory(&self) -> Arc<dyn HandlerFactory> {
        Arc::new(HttpingHandlerFactory {
            base: Arc::new(self.base.clone()),
            colo_filters: Arc::new(self.colo_filters.clone()),
            urls: Arc::new(self.urlist.clone()),
            url_index: Arc::new(AtomicUsize::new(0)),
            use_https: self.use_https,
            interface: self.base.args.interface.clone(),
        })
    }

    pub async fn run(self) -> Result<PingDelaySet, io::Error> {
        // 检查HTTPS模式下URL列表是否为空
        if self.use_https && self.urlist.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "警告：URL列表为空",
            ));
        }

        let handler_factory = self.make_handler_factory();
        common::run_ping_test(&self.base, handler_factory).await
    }
}
