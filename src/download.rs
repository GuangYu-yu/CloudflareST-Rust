use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use http_body::Body;

// 统一的速度更新间隔（毫秒）
const SPEED_UPDATE_INTERVAL_MS: u64 = 500;

// 下载测速相关常量
const TTFB_TIMEOUT_MS: u64 = 1200; // 首字节超时时间（毫秒）
const CONNECT_TIMEOUT_MS: u64 = 2000; // 连接超时时间（毫秒）
const WARM_UP_DURATION_SECS: u64 = 3; // 预热时间（秒）

use crate::args::Args;
use crate::common::{self, PingData};
use crate::progress::Bar;
use crate::warning_println;
use crate::hyper::{parse_url_to_uri, RequestContext};

// 定义下载处理器来处理下载数据
struct DownloadHandler {
    data_received: u64,
    last_update: Instant,
    bar: Arc<Bar>,
    speed_samples: VecDeque<(Instant, u64)>,
}

impl DownloadHandler {
    fn new(bar: Arc<Bar>) -> Self {
        let now = Instant::now();
        Self {
            data_received: 0,
            last_update: now,
            bar,
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
        while self.speed_samples.front().is_some_and(|(time, _)| *time < window_start) {
            self.speed_samples.pop_front();
        }
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
            self.bar.set_suffix(format!("{:.2} MB/s", speed / 1024.0 / 1024.0));
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
    colo_filter: Arc<Vec<String>>,
    ping_results: Vec<PingData>,
    timeout_flag: Arc<AtomicBool>,
    request_context: Arc<RequestContext>,
}

impl<'a> DownloadTest<'a> {
    pub(crate) async fn new(
        args: &'a Args,
        ping_results: Vec<PingData>,
        timeout_flag: Arc<AtomicBool>,
    ) -> Self {
        // 解析 URL
        let (uri, host) = parse_url_to_uri(&args.url).unwrap();

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

        let tls_connector = crate::hyper::build_tls_connector().unwrap();
        let request_context = Arc::new(RequestContext {
            interface_config: args.interface_config.clone(),
            tls_connector,
            connect_timeout_ms: CONNECT_TIMEOUT_MS,
            ttfb_timeout_ms: TTFB_TIMEOUT_MS,
        });

        Self {
            args,
            uri,
            host,
            bar: Arc::new(Bar::new(ping_results.len(), "", "")),
            colo_filter: Arc::new(common::parse_colo_filters(&args.httping_cf_colo)),
            ping_results,
            timeout_flag,
            request_context,
        }
    }

    pub(crate) async fn test_download_speed(&mut self) -> Vec<PingData> {
        // 数据中心过滤条件
        let colo_filters = self.colo_filter.clone();

        let mut ping_queue = self.ping_results.drain(..).collect::<VecDeque<_>>();
        let mut qualified_results = Vec::with_capacity(self.args.test_count);
        let mut tested_count = 0;

        let uri = &self.uri;
        let host = &self.host;

        // 初始化进度条显示（合格数|已测数）
        self.bar.update(qualified_results.len(), format!("{}|{}", qualified_results.len(), tested_count), "");

        while let Some(mut ping_result) = ping_queue.pop_front() {
            // 检查是否收到超时信号或已经找到足够数量的合格结果
            if common::check_timeout_signal(&self.timeout_flag)
                || qualified_results.len() >= self.args.test_count
            {
                break;
            }

            // 获取IP地址和检查是否需要获取 colo
            let need_colo = ping_result.data_center.is_none();

            // 执行下载测速
            let conn = DownloadConnection {
                uri: uri.clone(),
                host,
                addr: ping_result.addr,
            };
            
            let behavior = DownloadBehavior {
                duration: self.args.timeout_duration.unwrap(),
                need_colo,
                colo_filters: colo_filters.clone(),
            };
            
            let context = DownloadContext {
                timeout_flag: self.timeout_flag.clone(),
                request_context: self.request_context.clone(),
                bar: self.bar.clone(),
            };
            
            let (speed, maybe_colo) = download_handler(conn, behavior, &context).await;

            // 更新下载速度和可能的数据中心信息
            ping_result.download_speed = speed;

            if ping_result.data_center.is_none()
                && let Some(colo) = maybe_colo {
                ping_result.data_center = Some(colo);
            }

            // 检查速度是否符合要求
            let speed_match = match speed {
                Some(s) => s >= self.args.min_speed * 1024.0 * 1024.0,
                None => false,
            };

            // 检查数据中心是否符合要求
            let colo_match = colo_filters.is_empty() || common::is_colo_matched(ping_result.colo_str(), &colo_filters);

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
            }

            // 生成消息（合格数|已测数）
            let message = format!("{qualified_len}|{tested_count}");
            bar.update(tested_count, message, "");
        }

        // 进度条最后显示 Done!
        self.bar.set_suffix("Done!");

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

pub(crate) struct DownloadConnection<'a> {
    pub uri: http::Uri,
    pub host: &'a str,
    pub addr: SocketAddr,
}

pub(crate) struct DownloadBehavior {
    pub duration: Duration,
    pub need_colo: bool,
    pub colo_filters: Arc<Vec<String>>,
}

pub(crate) struct DownloadContext {
    pub timeout_flag: Arc<AtomicBool>,
    pub request_context: Arc<RequestContext>,
    pub bar: Arc<Bar>,
}

// 下载测速处理函数
async fn download_handler(
    conn: DownloadConnection<'_>,
    behavior: DownloadBehavior,
    context: &DownloadContext,
) -> (Option<f32>, Option<[u8; 3]>) {
    let DownloadConnection { uri, host, addr } = conn;
    let DownloadBehavior { duration: download_duration, need_colo, colo_filters } = behavior;

    let mut data_center = None;

    let warm_up_duration = Duration::from_secs(WARM_UP_DURATION_SECS);
    let extended_duration = download_duration + warm_up_duration;

    let original_uri = uri;

    let mut handler = DownloadHandler::new(context.bar.clone());

    let Some(resp) = context.request_context.send_request(
        host,
        &original_uri,
        addr,
        &http::Method::GET,
    ).await else {
        return (None, None);
    };

    let avg_speed = {
        if need_colo {
            data_center = common::extract_data_center(resp.headers());
            if data_center.is_none() {
                return (None, None);
            }
            if !colo_filters.is_empty() && !common::is_colo_matched(data_center.as_ref().map_or("", |b| std::str::from_utf8(b).unwrap()), &colo_filters) {
                return (None, data_center);
            }
        }

        let time_start = Instant::now();
        let mut total_content_read: u64 = 0;
        let mut actual_content_read: u64 = 0;
        let mut stream_ended = false;
        let mut stream_end_time = time_start;

        let mut body = resp.into_body();
        let mut body_pin = std::pin::Pin::new(&mut body);

        loop {
            let elapsed = time_start.elapsed();
            if elapsed >= extended_duration || context.timeout_flag.load(Ordering::SeqCst) {
                break;
            }

            match std::future::poll_fn(|cx| body_pin.as_mut().poll_frame(cx)).await {
                Some(Ok(frame)) => {
                    if let Some(data) = frame.data_ref() {
                        handler.update_data_received(data.len() as u64);
                        total_content_read += data.len() as u64;

                        let elapsed = time_start.elapsed();
                        if elapsed >= warm_up_duration {
                            actual_content_read += data.len() as u64;
                        }
                    }
                }
                Some(Err(_)) => return (None, data_center),
                None => {
                    stream_ended = true;
                    stream_end_time = Instant::now();
                    break;
                },
            }
        }

        let (bytes, duration) = if stream_ended {
            // 文件正常读完：总数据量 / 总耗时（含预热）
            let elapsed = stream_end_time.duration_since(time_start).as_secs_f32();
            (total_content_read as f32, elapsed)
        } else {
            // 超时退出：预热后数据量 / 设定测速时长
            let duration_secs = download_duration.as_secs_f32();
            (actual_content_read as f32, duration_secs)
        };

        // 超时退出时需要额外检查 bytes > 0，正常结束不需要
        let is_valid = duration > 0.0 && (stream_ended || bytes > 0.0);

        if is_valid {
            Some(bytes / duration)
        } else {
            None
        }
    };

    (avg_speed, data_center)
}