use std::{
    borrow::Cow,
    io::{stdout, Write},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicPtr, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use terminal_size::{terminal_size, Width};

/// 零开销无锁字符串
pub struct LockFreeString {
    ptr: AtomicPtr<Arc<str>>,
}

impl LockFreeString {
    pub fn new() -> Self {
        let arc_str: Arc<str> = Arc::from("");
        let boxed = Box::new(arc_str);
        Self {
            ptr: AtomicPtr::new(Box::into_raw(boxed)),
        }
    }

    pub fn set(&self, s: impl AsRef<str>) {
        let arc_str: Arc<str> = Arc::from(s.as_ref());
        let boxed = Box::new(arc_str);
        let new_ptr = Box::into_raw(boxed);
        let old_ptr = self.ptr.swap(new_ptr, Ordering::SeqCst);
        unsafe { let _ = Box::from_raw(old_ptr); }
    }

    pub fn get(&self) -> Arc<str> {
        let ptr = self.ptr.load(Ordering::SeqCst);
        unsafe { (*ptr).clone() } // clone Arc，不复制内容
    }
}

impl Drop for LockFreeString {
    fn drop(&mut self) {
        let ptr = self.ptr.load(Ordering::SeqCst);
        if !ptr.is_null() {
            unsafe { let _ = Box::from_raw(ptr); }
        }
    }
}

unsafe impl Send for LockFreeString {}
unsafe impl Sync for LockFreeString {}

/// 无锁进度条
pub struct Bar {
    pos: Arc<AtomicU64>,
    msg: Arc<LockFreeString>,
    prefix: Arc<LockFreeString>,
    is_done: Arc<AtomicBool>,
    thread_handle: Option<thread::JoinHandle<()>>,
}

impl Bar {
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

        let thread_handle = Some(thread::spawn(move || {
            let symbols = ["↖", "↗", "↘", "↙"];

            let draw = || {
                let term_width = terminal_size().map(|(Width(w), _)| w).unwrap_or(80) as usize;
                let reserved_space = 20 + start_str.len() + end_str.len() + 10;
                let bar_length = term_width.saturating_sub(reserved_space);

                let current_pos = pos_clone.load(Ordering::Relaxed) as usize;
                let progress = (current_pos.min(count as usize)) as f64 / count.max(1) as f64;
                let filled = (progress * bar_length as f64) as usize;
                let empty = bar_length.saturating_sub(filled);

                let tick_idx = ((start_instant.elapsed().as_millis() / 250) % 4) as usize;
                let is_done = is_done_clone.load(Ordering::Relaxed);

                let msg_str = msg_clone.get();
                let prefix_str = prefix_clone.get();

                if !is_done {
                    print!("\x1b[?25l"); // 隐藏光标
                }

                // 绘制进度条
                print!("\r\x1b[33m{}\x1b[0m [", msg_str);
                if filled > 0 {
                    print!("\x1b[34m{}{}\x1b[0m", "=".repeat(filled - 1), symbols[tick_idx]);
                } else {
                    print!("\x1b[34m{}\x1b[0m", symbols[tick_idx]);
                }
                print!("\x1b[34m{}\x1b[0m] ", "_".repeat(empty));
                print!("{} \x1b[32m{}\x1b[0m", start_str, prefix_str);
                if !end_str.is_empty() {
                    print!(" \x1b[32m{}\x1b[0m", end_str);
                }

                if is_done {
                    print!("\x1b[?25h\n"); // 恢复光标并换行
                    stdout().flush().ok();
                } else {
                    stdout().flush().ok();
                }
            };

            while !is_done_clone.load(Ordering::Relaxed) {
                draw();
                thread::sleep(Duration::from_millis(120));
            }
            draw();
        }));

        Self {
            pos,
            msg,
            prefix,
            is_done,
            thread_handle,
        }
    }

    pub fn grow(&self, num: u64, msg: impl Into<Cow<'static, str>>) {
        if self.is_done.load(Ordering::Relaxed) {
            return;
        }
        self.pos.fetch_add(num, Ordering::Relaxed);
        self.msg.set(msg.into().as_ref());
    }

    pub fn set_suffix(&self, suffix: impl Into<Cow<'static, str>>) {
        if self.is_done.load(Ordering::Relaxed) {
            return;
        }
        self.prefix.set(suffix.into().as_ref());
    }

    pub fn set_message(&self, message: impl Into<Cow<'static, str>>) {
        if self.is_done.load(Ordering::Relaxed) {
            return;
        }
        self.msg.set(message.into().as_ref());
    }

    pub fn done_at_current_pos(&self) {
        self.is_done.store(true, Ordering::Relaxed);
    }
}

impl Drop for Bar {
    fn drop(&mut self) {
        self.done_at_current_pos();
        if let Some(handle) = self.thread_handle.take() {
            handle.join().ok();
        }
        print!("\x1b[?25h"); // 确保光标恢复
    }
}