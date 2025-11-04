use std::{
    borrow::Cow,
    io::{stdout, Write},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicPtr, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use terminal_size::{terminal_size, Width};

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
        5 | _ => (v, p, q),
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
            ptr: AtomicPtr::new(Box::into_raw(Box::new(Arc::from("")))),
        }
    }

    pub fn set(&self, s: &str) {
        let old_ptr = self.ptr.swap(Box::into_raw(Box::new(Arc::from(s))), Ordering::SeqCst);
        unsafe { drop(Box::from_raw(old_ptr)); }
    }

    pub fn get(&self) -> Arc<str> {
        let ptr = self.ptr.load(Ordering::SeqCst);
        unsafe { (*ptr).clone() }
    }
}

impl Drop for LockFreeString {
    fn drop(&mut self) {
        let ptr = self.ptr.load(Ordering::SeqCst);
        if !ptr.is_null() {
            unsafe { drop(Box::from_raw(ptr)); }
        }
    }
}

unsafe impl Send for LockFreeString {}
unsafe impl Sync for LockFreeString {}

// 进度条
pub struct Bar {
    pos: Arc<AtomicU64>,
    msg: Arc<LockFreeString>,
    prefix: Arc<LockFreeString>,
    is_done: Arc<AtomicBool>,
    thread_handle: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
}

impl Bar {
    fn update_if_not_done<F: FnOnce()>(&self, f: F) {
        if !self.is_done.load(Ordering::Relaxed) {
            f();
        }
    }

    pub fn new(count: u64, start_str: &str, end_str: &str) -> Self {
        let pos = Arc::new(AtomicU64::new(0));
        let msg = Arc::new(LockFreeString::new());
        let prefix = Arc::new(LockFreeString::new());
        let is_done = Arc::new(AtomicBool::new(false));
        let start_instant = Instant::now();

        let pos_clone = Arc::clone(&pos);
        let msg_clone = Arc::clone(&msg);
        let prefix_clone = Arc::clone(&prefix);
        let is_done_clone = Arc::clone(&is_done);

        let start_str = start_str.to_string();
        let end_str = end_str.to_string();

        // 固定终端宽度和 reserved_space，避免每次刷新都计算
        let term_width = terminal_size().map(|(Width(w), _)| w).unwrap_or(80) as usize;
        let reserved_space = 20 + start_str.len() + end_str.len() + 10;

        let handle = thread::spawn(move || {
            let mut first_draw = true;
            loop {
                let bar_length = term_width.saturating_sub(reserved_space);
                let current_pos = pos_clone.load(Ordering::Relaxed);
                let progress = (current_pos.min(count) as f64) / count.max(1) as f64;
                let filled = (progress * bar_length as f64) as usize;
                let phase = (start_instant.elapsed().as_secs_f64() * 0.3) % 1.0;

                let percent_str = format!("[{:>4.2}%]", progress * 100.0);
                let percent_chars: Vec<char> = percent_str.chars().collect();
                let start_index = (bar_length / 2).saturating_sub(percent_chars.len() / 2)
                    .min(bar_length.saturating_sub(percent_chars.len()));

                let mut bar_str = String::with_capacity(bar_length * 10);
                for i in 0..bar_length {
                    let mut c = ' ';
                    if i < filled { c = '▇'; }
                    if i >= start_index && i < start_index + percent_chars.len() {
                        c = percent_chars[i - start_index];
                    }
                    if i < filled && c == '▇' {
                        let hue = (1.0 - i as f64 / bar_length as f64 + phase) % 1.0;
                        let (r, g, b) = hsv_to_rgb(hue, 0.4, 0.5);
                        bar_str.push_str(&format!("\x1b[38;2;{};{};{}m▇\x1b[0m", r, g, b));
                    } else {
                        bar_str.push(c);
                    }
                }

                let msg_str = msg_clone.get();
                let prefix_str = prefix_clone.get();
                let is_done_val = is_done_clone.load(Ordering::Relaxed);

                if first_draw {
                    print!("\x1b[?25l");
                    first_draw = false;
                }

                print!(
                    "\r\x1b[33m{}\x1b[0m [{}] {} \x1b[32m{}\x1b[0m {}",
                    msg_str, bar_str, start_str, prefix_str, end_str
                );

                if is_done_val {
                    print!("\x1b[?25h\n");
                }

                stdout().flush().ok();

                if is_done_clone.load(Ordering::Acquire) { break; }
                thread::sleep(Duration::from_millis(120));
            }
        });

        let thread_handle = Arc::new(Mutex::new(Some(handle)));

        Self { pos, msg, prefix, is_done, thread_handle }
    }

    pub fn grow(&self, num: u64, msg: impl Into<Cow<'static, str>>) {
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

    pub fn done(&self) {
        self.is_done.store(true, Ordering::Release);
        if let Ok(mut guard) = self.thread_handle.lock() {
            if let Some(handle) = guard.take() {
                handle.join().ok();
            }
        }
        print!("\x1b[?25h");
        let _ = stdout().flush();
    }
}

impl Drop for Bar {
    fn drop(&mut self) {
        self.done();
    }
}
