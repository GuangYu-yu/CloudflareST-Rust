use std::{
    fmt::Write as FmtWrite,
    io::{self, stdout, Write},
    sync::{
        atomic::{AtomicBool, AtomicPtr, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

// 进度条颜色配置
const PROGRESS_BAR_BRIGHTNESS: [f64; 2] = [
    0.5, // 亮度基准值
    0.3  // 亮度变化幅度
];

// 进度条动画配置常量
const PROGRESS_BAR_SPEED: f64 = 0.2; // 进度条色彩流动速度
const WAVE_WIDTH: f64 = 16.0; // 波浪效果宽度
const SPEED_FACTOR: f64 = 0.3; // 波浪移动速度因子
const SATURATION_BASE: f64 = 0.6; // 饱和度基准值
const REFRESH_INTERVAL_MS: u64 = 40; // 进度条刷新间隔（毫秒）

// 状态快照：打包所有状态，保证原子更新时的一致性
struct Snapshot {
    pos: usize,
    msg: Arc<str>,
    prefix: Arc<str>,
}

// 内部状态：所有实际数据都在这里
struct BarInner {
    state: AtomicPtr<Snapshot>,
    is_done: AtomicBool,
    total: usize,
    start_str: Arc<str>,
    end_str: Arc<str>,
}

// 外层 Handle：对外暴露的接口
pub(crate) struct Bar {
    inner: Arc<BarInner>,
}

unsafe impl Send for BarInner {}
unsafe impl Sync for BarInner {}

impl BarInner {
    // -------------------------------------------------------------------------
    // 读接口：渲染逻辑（完全保留 log.txt 的视觉效果）
    // -------------------------------------------------------------------------

    fn run_render_loop(&self) {
        let mut stdout_handle = stdout();
        let start_instant = Instant::now();
        let mut bar_str = String::new();
        let mut output_buffer = String::new();

        while !self.is_done.load(Ordering::Acquire) {
            self.render_once(&mut stdout_handle, &start_instant, &mut bar_str, &mut output_buffer);
            thread::sleep(Duration::from_millis(REFRESH_INTERVAL_MS));
        }

        self.render_once(&mut stdout_handle, &start_instant, &mut bar_str, &mut output_buffer);
        let _ = writeln!(stdout_handle);
    }

    fn render_once(
        &self,
        stdout_handle: &mut io::Stdout,
        start_instant: &Instant,
        bar_str: &mut String,
        output_buffer: &mut String,
    ) {
        let ptr = self.state.load(Ordering::Acquire);
        if ptr.is_null() { return; }

        unsafe {
            Arc::increment_strong_count(ptr);
            let snap = Arc::from_raw(ptr);

            bar_str.clear();
            output_buffer.clear();

            // 定义未填充区域的亮灰色背景色 (RGB: 70, 70, 70)
            const UNFILLED_BG: (u8, u8, u8) = (70, 70, 70);

            // 在循环内重新获取终端宽度
            let term_width = get_terminal_width();
            let reserved_space = 20 + self.start_str.len() + self.end_str.len() + 10;
            let bar_length = term_width.saturating_sub(reserved_space);

            let current_pos = snap.pos;
            let progress = (current_pos.min(self.total) as f64) / self.total.max(1) as f64;
            let filled = (progress * bar_length as f64) as usize;
            let phase = (start_instant.elapsed().as_secs_f64() * PROGRESS_BAR_SPEED) % 1.0;

            // 百分比
            let percent_content = format!(" {:>4.1}% ", progress * 100.0);
            let percent_chars: Vec<char> = percent_content.chars().collect();
            let percent_len = percent_chars.len();

            let start_index = (bar_length / 2).saturating_sub(percent_len / 2)
                .min(bar_length.saturating_sub(percent_len));
            let end_index = start_index + percent_len;

            // 预留足够的容量
            bar_str.reserve(bar_length * 10);

            for i in 0..bar_length {
                let is_filled = i < filled;
                let is_percent_char = i >= start_index && i < end_index;
                let percent_index = if is_percent_char { i - start_index } else { 0 };

                // 1. 计算已完成部分的颜色 (进度条颜色)
                let hue = (1.0 - i as f64 / bar_length as f64 + phase) % 1.0;

                // 计算周期性变化的饱和度和亮度 (波浪效果)
                let t = (start_instant.elapsed().as_secs_f64() * SPEED_FACTOR).fract();
                let bar_length_f64 = bar_length as f64;

                // 1. 计算波峰中心位置 (mu)
                let mu = t * bar_length_f64; // 单向流动：t 从 0.0 到 1.0 匀速增长
                let i_f64 = i as f64;

                // 2. 计算周期性的最短距离
                let distance_raw = (i_f64 - mu).abs();
                // 3. 计算环绕距离 (即通过另一侧边界的最短距离)
                let distance_wrap = bar_length_f64 - distance_raw;

                // 4. 周期性距离：取直接距离和环绕距离的最小值
                let distance = distance_raw.min(distance_wrap);

                // 5. 使用周期性距离计算衰减 (高斯函数)
                let distance_ratio = distance / WAVE_WIDTH;
                let attenuation = (-distance_ratio * distance_ratio).exp();
                let brightness = PROGRESS_BAR_BRIGHTNESS[0] + PROGRESS_BAR_BRIGHTNESS[1] * attenuation;
                // 饱和度衰减：波峰中心保持 SATURATION_BASE，边缘略微降低
                let saturation = SATURATION_BASE * (0.6 + 0.4 * attenuation);
                let (r, g, b) = hsv_to_rgb(hue, saturation, brightness);

                // 2. 确定当前单元格应该使用的背景色
                let (bg_r, bg_g, bg_b) = if is_filled {
                    (r, g, b) // 已完成：使用动态彩色
                } else {
                    UNFILLED_BG // 未完成：使用亮灰色
                };

                // 写入 ANSI 颜色码和字符
                if is_percent_char {
                    // 3. 百分比字符：设置背景色 + 亮白前景
                    let c = percent_chars[percent_index];
                    let _ = write!(
                        bar_str,
                        "\x1b[48;2;{};{};{}m\x1b[1;97m{}\x1b[0m",
                        bg_r, bg_g, bg_b, c
                    );
                } else {
                    // 4. 普通进度块：设置背景色 + 空格前景
                    let _ = write!(
                        bar_str,
                        "\x1b[48;2;{};{};{}m \x1b[0m",
                        bg_r, bg_g, bg_b
                    );
                }
            }

            // 将所有输出内容写入 output_buffer
            let _ = write!(
                output_buffer,
                "\r\x1b[K\x1b[33m{}\x1b[0m {} {} \x1b[32m{}\x1b[0m {}",
                snap.msg, bar_str, self.start_str, snap.prefix, self.end_str
            );

            // 一次性原子写入所有内容
            if let Err(e) = stdout_handle.write_all(output_buffer.as_bytes())
                && e.kind() == io::ErrorKind::BrokenPipe {
                return;
            }

            let _ = stdout_handle.flush();
        }
    }
}

unsafe impl Send for Bar {}
unsafe impl Sync for Bar {}

impl Bar {
    pub(crate) fn new(count: usize, start_str: &str, end_str: &str) -> Self {
        let initial_snapshot = Arc::new(Snapshot {
            pos: 0,
            msg: "".into(),
            prefix: "".into(),
        });

        let inner = Arc::new(BarInner {
            state: AtomicPtr::new(Arc::into_raw(initial_snapshot) as *mut Snapshot),
            is_done: AtomicBool::new(false),
            total: count.max(1),
            start_str: Arc::from(start_str),
            end_str: Arc::from(end_str),
        });

        let inner_clone = Arc::clone(&inner);
        thread::spawn(move || {
            inner_clone.run_render_loop();
        });

        Self { inner }
    }

    // -------------------------------------------------------------------------
    // 写接口：无锁快照更新
    // -------------------------------------------------------------------------

    // 内部提交逻辑，保持代码 DRY
    fn commit(&self, new_snap: Arc<Snapshot>) {
        let new_ptr = Arc::into_raw(new_snap) as *mut Snapshot;
        let old_ptr = self.inner.state.swap(new_ptr, Ordering::AcqRel);
        if !old_ptr.is_null() {
            unsafe { Arc::from_raw(old_ptr); }
        }
    }

    pub(crate) fn update(&self, pos: usize, msg: impl Into<Arc<str>>, suffix: impl Into<Arc<str>>) {
        if self.inner.is_done.load(Ordering::Relaxed) { return; }

        let new_snapshot = Arc::new(Snapshot {
            pos,
            msg: msg.into(),
            prefix: suffix.into(),
        });
        self.commit(new_snapshot);
    }

    pub(crate) fn set_suffix(&self, suffix: impl Into<Arc<str>>) {
        let ptr = self.inner.state.load(Ordering::Acquire);
        unsafe {
            let new = Arc::new(Snapshot {
                pos: (*ptr).pos,
                msg: (*ptr).msg.clone(),
                prefix: suffix.into(),
            });
            self.commit(new);
        }
    }

    pub(crate) fn done(&self) {
        self.inner.is_done.store(true, Ordering::Release);
        
        while Arc::strong_count(&self.inner) > 1 {
            thread::yield_now();
        }
        
        let _ = stdout().flush();
    }
}

impl Drop for Bar {
    fn drop(&mut self) {
        self.done();
        let ptr = self.inner.state.load(Ordering::Acquire);
        if !ptr.is_null() {
            unsafe { Arc::from_raw(ptr); }
        }
    }
}

// HSV 到 RGB 颜色转换
fn hsv_to_rgb(h: f64, s: f64, v: f64) -> (u8, u8, u8) {
    let i = (h * 6.0).floor() as i32;
    let f = h * 6.0 - i as f64;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match i % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

// 获取终端宽度，返回usize类型
fn get_terminal_width() -> usize {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::System::Console::{GetConsoleScreenBufferInfo, GetStdHandle, STD_OUTPUT_HANDLE, CONSOLE_SCREEN_BUFFER_INFO};
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        unsafe {
            let handle = GetStdHandle(STD_OUTPUT_HANDLE);
            if handle == 0 as _ || handle == INVALID_HANDLE_VALUE {
                return 80;
            }
            let mut csbi: CONSOLE_SCREEN_BUFFER_INFO = std::mem::zeroed();
            if GetConsoleScreenBufferInfo(handle, &mut csbi) != 0 {
                let w = csbi.srWindow.Right - csbi.srWindow.Left + 1;
                return w as usize;
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use libc::{ioctl, winsize, TIOCGWINSZ, STDOUT_FILENO};
        unsafe {
            let mut ws: winsize = std::mem::zeroed();
            if ioctl(STDOUT_FILENO, TIOCGWINSZ, &mut ws) == 0 {
                return ws.ws_col as usize;
            }
        }
    }

    80
}
