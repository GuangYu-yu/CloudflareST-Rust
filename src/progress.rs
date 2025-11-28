use std::{
    borrow::Cow,
    fmt::Write as FmtWrite,
    io::{self, stdout, Write},
    sync::{
        atomic::{AtomicBool, AtomicUsize, AtomicPtr, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use terminal_size::{terminal_size, Width};

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

// 无锁字符串
pub struct LockFreeString {
    ptr: AtomicPtr<Arc<str>>,
}

impl LockFreeString {
    pub fn new() -> Self {
        Self {
            // 使用 Box::into_raw 传递指针给 AtomicPtr
            ptr: AtomicPtr::new(Box::into_raw(Box::new(Arc::from("")))),
        }
    }

    pub fn set(&self, s: &str) {
        // 创建新的 Arc<str>，封装在 Box 中
        let new_box = Box::new(Arc::from(s));
        let new_ptr = Box::into_raw(new_box);

        // 原子交换指针
        let old_ptr = self.ptr.swap(new_ptr, Ordering::SeqCst);
        
        // 释放旧的 Box<Arc<str>>
        unsafe { drop(Box::from_raw(old_ptr)); }
    }

    pub fn get(&self) -> Arc<str> {
        let ptr = self.ptr.load(Ordering::SeqCst);
        // 克隆 Arc<str>，增加引用计数，保证数据存活
        unsafe { (*ptr).clone() } 
    }
}

impl Drop for LockFreeString {
    fn drop(&mut self) {
        let ptr = self.ptr.load(Ordering::SeqCst);
        if !ptr.is_null() {
            // 释放 Arc<str> 所在的 Box
            unsafe { drop(Box::from_raw(ptr)); }
        }
    }
}

unsafe impl Send for LockFreeString {}
unsafe impl Sync for LockFreeString {}

// 进度条
pub struct Bar {
    pos: Arc<AtomicUsize>,
    msg: Arc<LockFreeString>,
    prefix: Arc<LockFreeString>,
    is_done: Arc<AtomicBool>,
    thread_handle: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
}

// 获取终端宽度，返回usize类型
fn get_terminal_width() -> usize {
    terminal_size()
        .map(|(Width(w), _)| w as usize)
        .unwrap_or(80)
}

impl Bar {
    // 仅在进度条未完成时执行操作
    fn update_if_not_done<F: FnOnce()>(&self, f: F) {
        if !self.is_done.load(Ordering::Relaxed) {
            f();
        }
    }

    pub fn new(count: usize, start_str: &str, end_str: &str) -> Self {
        let pos = Arc::new(AtomicUsize::new(0));
        let msg = Arc::new(LockFreeString::new());
        let prefix = Arc::new(LockFreeString::new());
        let is_done = Arc::new(AtomicBool::new(false));
        let start_instant = Instant::now();

        let pos_clone = Arc::clone(&pos);
        let msg_clone = Arc::clone(&msg);
        let prefix_clone = Arc::clone(&prefix);
        let is_done_clone = Arc::clone(&is_done);

        let start_str_arc: Arc<str> = start_str.into();
        let end_str_arc: Arc<str> = end_str.into();

        let handle = thread::spawn(move || {
            // 定义未填充区域的亮灰色背景色 (RGB: 70, 70, 70)
            const UNFILLED_BG: (u8, u8, u8) = (70, 70, 70);
            
            // 在循环外创建，以重用内存分配
            let mut bar_str = String::new();
            let mut output_buffer = String::new();
            
            loop {
                bar_str.clear();
                output_buffer.clear(); // 清空缓冲区以重用内存

                // 在循环内重新获取终端宽度
                let term_width = get_terminal_width();
                let reserved_space = 20 + start_str_arc.len() + end_str_arc.len() + 10;
                let bar_length = term_width.saturating_sub(reserved_space);
                
                let current_pos = pos_clone.load(Ordering::Relaxed);
                let progress = (current_pos.min(count) as f64) / count.max(1) as f64;
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
                            &mut bar_str,
                            "\x1b[48;2;{};{};{}m\x1b[1;97m{}\x1b[0m",
                            bg_r, bg_g, bg_b, c
                        );
                    } else {
                        // 4. 普通进度块：设置背景色 + 空格前景
                        let _ = write!(
                            &mut bar_str,
                            "\x1b[48;2;{};{};{}m \x1b[0m",
                            bg_r, bg_g, bg_b
                        );
                    }
                }

                let msg_str = msg_clone.get();
                let prefix_str = prefix_clone.get();
                let is_done_val = is_done_clone.load(Ordering::Relaxed);

                // 将所有输出内容写入 output_buffer
                let _ = write!(
                    &mut output_buffer,
                    "\r\x1b[K\x1b[33m{}\x1b[0m {} {} \x1b[32m{}\x1b[0m {}",
                    msg_str, bar_str, start_str_arc, prefix_str, end_str_arc
                );

                if is_done_val {
                    output_buffer.push('\n');
                }

                // 一次性原子写入所有内容
                if let Err(e) = stdout().write_all(output_buffer.as_bytes())
                    && e.kind() == io::ErrorKind::BrokenPipe {
                    break; 
                }
                
                let _ = stdout().flush();

                if is_done_clone.load(Ordering::Acquire) { break; }
                thread::sleep(Duration::from_millis(REFRESH_INTERVAL_MS));
            }
        });

        let thread_handle = Arc::new(Mutex::new(Some(handle)));

        Self { pos, msg, prefix, is_done, thread_handle }
    }

    pub fn grow(&self, num: usize, msg: impl Into<Cow<'static, str>>) {
        self.update_if_not_done(|| {
            self.pos.fetch_add(num, Ordering::Relaxed);
            self.msg.set(msg.into().as_ref());
        });
    }

    pub fn set_suffix(&self, suffix: impl Into<Cow<'static, str>>) {
        self.update_if_not_done(|| self.prefix.set(suffix.into().as_ref()));
    }

    pub fn set_message(&self, message: impl Into<Cow<'static, str>>) {
        self.update_if_not_done(|| self.msg.set(message.into().as_ref()));
    }

    // 原子更新所有进度条数据，确保一致性
    pub fn update_all(&self, num: usize, message: impl Into<Cow<'static, str>>, suffix: impl Into<Cow<'static, str>>) {
        self.update_if_not_done(|| {
            self.pos.fetch_add(num, Ordering::Relaxed);
            self.msg.set(message.into().as_ref());
            self.prefix.set(suffix.into().as_ref());
        });
    }

    // 完成进度条并清理
    pub fn done(&self) {
        // 原子设置完成标志
        self.is_done.store(true, Ordering::Release);
        
        // 尝试获取锁并 join 渲染线程
        if let Ok(mut guard) = self.thread_handle.lock() && let Some(handle) = guard.take() {
            // 忽略 join 错误
            handle.join().ok(); 
        }
        let _ = stdout().flush();
    }
}

// 确保在 Bar 实例被 drop 时调用 done()
impl Drop for Bar {
    fn drop(&mut self) {
        self.done();
    }
}