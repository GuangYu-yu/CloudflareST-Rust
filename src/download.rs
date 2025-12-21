use std::cmp::min;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use http_body::Body;

// 统一的速度更新间隔（毫秒）
const SPEED_UPDATE_INTERVAL_MS: u64 = 500;

// 下载测速相关常量
const TTFB_TIMEOUT_MS: u64 = 1200; // 首字节超时时间（毫秒）
const WARM_UP_DURATION_SECS: u64 = 3; // 预热时间（秒）

use crate::args::Args;
use crate::common::{self, PingData};
use crate::progress::Bar;
use crate::warning_println;
use crate::hyper::{self, parse_url_to_uri};

// 定义下载处理器来处理下载数据
struct DownloadHandler {
    data_received: u64,
    last_update: Instant,
    current_speed: Arc<Mutex<f32>>,
    speed_samples: VecDeque<(Instant, u64)>,
}

impl DownloadHandler {
    fn new(current_speed: Arc<Mutex<f32>>) -> Self {
        let now = Instant::now();
        Self {
            data_received: 0,
            last_update: now,
            current_speed,
            speed_samples: VecDeque::new(),
        }
    }

    // 添加数据点
    fn add_data_point(&mut self, size: u64) {
        self.data_received += size;
        self.speed_samples.push_back((Instant::now(), self.data_received));
    }

    // 清理超出时间窗口的数据点
    fn cleanup_old_samples(&mut self, window_start: Instant) {
        self.speed_samples.retain(|&(time, _)| time >= window_start);
    }

    // 纯函数计算速度
    fn calculate_speed(&self) -> f32 {
        self.speed_samples
            .front()
            .zip(self.speed_samples.back())
            .and_then(|(first, last)| {
                let bytes_diff = last.1 - first.1;
                let time_diff = last.0.duration_since(first.0).as_secs_f32();
                if bytes_diff == 0 || time_diff <= 0.0 {
                    None
                } else {
                    Some(bytes_diff as f32 / time_diff)
                }
            })
            .unwrap_or(0.0)
    }

    // 检查是否需要更新显示
    fn should_update_display(&self) -> bool {
        let now = Instant::now();
        now.duration_since(self.last_update).as_millis() >= SPEED_UPDATE_INTERVAL_MS as u128
    }

    // 更新显示速度
    fn update_display(&mut self) {
        if self.should_update_display() {
            let window_start = Instant::now() - Duration::from_millis(SPEED_UPDATE_INTERVAL_MS);
            self.cleanup_old_samples(window_start);
            
            let speed = self.calculate_speed();
            *self.current_speed.lock().unwrap() = speed;
            self.last_update = Instant::now();
        }
    }

    // 更新接收到的数据
    fn update_data_received(&mut self, size: u64) {
        self.add_data_point(size);
        self.update_display();
    }
}

pub(crate) struct DownloadTest<'a> {
    args: &'a Args,
    uri: http::Uri,
    host: String,
    bar: Arc<Bar>,
    current_speed: Arc<Mutex<f32>>,
    colo_filter: Arc<Vec<String>>,
    ping_results: Vec<PingData>,
    timeout_flag: Arc<AtomicBool>,
}

impl<'a> DownloadTest<'a> {
    pub(crate) async fn new(
        args: &'a Args,
        ping_results: Vec<PingData>,
        timeout_flag: Arc<AtomicBool>,
    ) -> Self {
        // 解析 URL
        let trace_url = args.url
            .find("://")
            .map(|_| args.url.to_owned())
            .unwrap_or_else(|| {
                let protocol = if args.httping_https { "https" } else { "http" };
                format!("{}://{}", protocol, args.url)
            });
        let (uri, host) = parse_url_to_uri(&trace_url).unwrap();

        // 计算实际需要测试的数量
        let test_num = min(args.test_count, ping_results.len());

        // 先检查队列数量是否足够
        if args.test_count > ping_results.len() {
            warning_println(format_args!("队列的 IP 数量不足，可能需要降低延迟测速筛选条件！"));
        }

        println!(
            "开始下载测速（下限：{:.2} MB/s, 所需：{}, 队列：{}）",
            args.min_speed,
            args.test_count,
            ping_results.len()
        );

        Self {
            args,
            uri,
            host,
            bar: Arc::new(Bar::new(test_num, "", "MB/s")),
            current_speed: Arc::new(Mutex::new(0.0)),
            colo_filter: Arc::new(common::parse_colo_filters(&args.httping_cf_colo)),
            ping_results,
            timeout_flag,
        }
    }

    pub(crate) async fn test_download_speed(&mut self) -> Vec<PingData> {
        // 数据中心过滤条件
        let colo_filters = Arc::clone(&self.colo_filter);

        let current_speed_arc: Arc<Mutex<f32>> = Arc::clone(&self.current_speed);
        let bar_arc = self.bar.clone();
        let timeout_flag_clone = Arc::clone(&self.timeout_flag);
        
        // 使用统一的速度更新间隔
        let update_interval = Duration::from_millis(SPEED_UPDATE_INTERVAL_MS);

        let speed_update_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(update_interval);
            
            loop {
                if timeout_flag_clone.load(Ordering::SeqCst) {
                    break;
                }
                
                // 锁定并读取当前速度 (B/s)
                let speed = *current_speed_arc.lock().unwrap();
                
                if speed >= 0.0 {
                    // 更新进度条的速率后缀 (MB/s)
                    bar_arc.set_suffix(format!("{:.2}", speed / 1024.0 / 1024.0));
                }

                interval.tick().await; // 等待下一个间隔
            }
        });

        let mut ping_queue = self.ping_results.drain(..).collect::<VecDeque<_>>();
        let mut qualified_results = Vec::with_capacity(self.args.test_count);
        let mut tested_count = 0;

        let uri = &self.uri;
        let host = &self.host;

        while let Some(mut ping_result) = ping_queue.pop_front() {
            // 检查是否收到超时信号或已经找到足够数量的合格结果
            if common::check_timeout_signal(&self.timeout_flag)
                || qualified_results.len() >= self.args.test_count
            {
                break;
            }

            // 获取IP地址和检查是否需要获取 colo
            let need_colo = ping_result.data_center.is_empty();

            // 执行下载测速
            let params = DownloadHandlerParams {
                addr: ping_result.addr,
                uri: uri.clone(),
                host,
                download_duration: self.args.timeout_duration.unwrap(),
                current_speed: Arc::clone(&self.current_speed),
                need_colo,
                timeout_flag: Arc::clone(&self.timeout_flag),
                colo_filters: Arc::clone(&colo_filters),
                interface_config: &self.args.interface_config,
            };
            
            let (speed, maybe_colo) = download_handler(params).await;

            // 更新下载速度和可能的数据中心信息
            ping_result.download_speed = speed;

            if ping_result.data_center.is_empty()
                && let Some(colo) = maybe_colo {
                ping_result.data_center = colo;
            }

            // 检查速度是否符合要求
            let speed_match = match speed {
                Some(s) => s >= self.args.min_speed * 1024.0 * 1024.0,
                None => false,
            };

            // 检查数据中心是否符合要求
            let colo_match = colo_filters.is_empty() || common::is_colo_matched(&ping_result.data_center, &colo_filters);

            // 更新已测试计数
            tested_count += 1;

            // 同时满足速度和数据中心要求
            let bar = self.bar.as_ref();
            let mut qualified_len = qualified_results.len();
            
            let is_qualified = speed_match && colo_match;
            
            // 如果合格，先推入结果并更新长度
            if is_qualified {
                qualified_results.push(ping_result);
                qualified_len += 1;
                bar.grow(1, "");
            }

            // 生成消息（合格数 已测数）
            let message = format!("{}|{}", qualified_len, tested_count);
            bar.set_message(message);
        }

        // 中止速度更新任务
        speed_update_handle.abort();

        // 完成进度条但保持当前进度
        self.bar.done();

        // 如果没有找到足够的结果，打印提示
        if qualified_results.len() < self.args.test_count {
            warning_println(format_args!("下载测速符合要求的 IP 数量不足！"));
        }

        // 对结果进行业务排序
        common::sort_results(&mut qualified_results[..]);

        qualified_results
    }
}

// 下载测速参数结构体
struct DownloadHandlerParams<'a> {
    addr: SocketAddr,
    uri: http::Uri,
    host: &'a str,
    download_duration: Duration,
    current_speed: Arc<Mutex<f32>>,
    need_colo: bool,
    timeout_flag: Arc<AtomicBool>,
    colo_filters: Arc<Vec<String>>,
    interface_config: &'a Arc<crate::interface::InterfaceParamResult>,
}

// 下载测速处理函数
async fn download_handler(params: DownloadHandlerParams<'_>) -> (Option<f32>, Option<String>) {
    // 在每次新的下载开始前重置速度为0
    *params.current_speed.lock().unwrap() = 0.0;

    let mut data_center = None;

    // 定义连接和TTFB的超时
    let warm_up_duration = Duration::from_secs(WARM_UP_DURATION_SECS);
    let extended_duration = params.download_duration + warm_up_duration;

    // 创建客户端进行下载测速
    let client = match hyper::build_hyper_client(
        params.addr,
        params.interface_config,
        TTFB_TIMEOUT_MS,
    ) {
        Some(client) => client,
        None => return (None, None),
    };

    // 创建下载处理器
    let mut handler = DownloadHandler::new(params.current_speed.clone());

    // 发送GET请求
    let response = hyper::send_get_response(
        &client, 
        params.host, 
        params.uri,
        TTFB_TIMEOUT_MS
    ).await.ok();

    // 如果获取到响应，开始下载
    let avg_speed = if let Some(resp) = response {
        // 如果需要获取数据中心信息，从响应头中提取
        if params.need_colo {
            data_center = common::extract_data_center(&resp);
            // 如果没有提取到数据中心信息，直接返回None
            if data_center.is_none() {
                return (None, None);
            }
            // 如果数据中心不符合要求，速度返回None，数据中心正常返回
            if let Some(dc) = &data_center
                && !params.colo_filters.is_empty() && !common::is_colo_matched(dc, &params.colo_filters) {
                return (None, data_center);
            }
        }

        // 读取响应体
        let time_start = Instant::now();
        let mut actual_content_read: u64 = 0;
        let mut actual_start_time: Option<Instant> = None;
        let mut last_data_time: Option<Instant> = None; // 记录最后读取数据的时间
        
        let mut body = resp.into_body();
        let mut body_pin = std::pin::Pin::new(&mut body);
        
        loop {
            // 检查是否应该继续下载
            let elapsed = time_start.elapsed();
            if elapsed >= extended_duration || params.timeout_flag.load(Ordering::SeqCst) {
                break;
            }

            // 异步读取下一帧数据
            match std::future::poll_fn(|cx| body_pin.as_mut().poll_frame(cx)).await {
                Some(Ok(frame)) => {
                    if let Some(data) = frame.data_ref() {
                        let size = data.len() as u64;
                        handler.update_data_received(size);

                        let current_time = Instant::now();
                        let elapsed = current_time.duration_since(time_start);

                        // 如果已经过了预热时间，开始记录实际下载数据
                        if elapsed >= warm_up_duration {
                            if actual_start_time.is_none() {
                                actual_start_time = Some(current_time);
                            }
                            actual_content_read += size;
                            last_data_time = Some(current_time); // 更新最后数据时间
                        }
                    }
                }
                Some(Err(_)) => return (None, data_center), // 网络错误直接返回None
                None => break, // 没有更多数据
            }
        }

        // 计算实际速度（只计算预热后的数据）
        actual_start_time.and_then(|start| {
            let end_time = last_data_time.unwrap_or_else(Instant::now); // 使用最后数据时间
            let actual_elapsed = end_time.duration_since(start).as_secs_f32();
            if actual_elapsed > 0.0 {
                Some(actual_content_read as f32 / actual_elapsed)
            } else {
                None
            }
        })
    } else {
        None
    };

    (avg_speed, data_center)
}