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

// 无锁字符串结构，用于多线程环境下安全地共享字符串数据
pub struct LockFreeString {
    ptr: AtomicPtr<Arc<str>>,
}

impl LockFreeString {
    // 创建新的空字符串
    pub fn new() -> Self {
        let arc_str: Arc<str> = Arc::from("");
        let boxed = Box::new(arc_str);
        Self {
            ptr: AtomicPtr::new(Box::into_raw(boxed)),
        }
    }

    // 设置字符串值
    pub fn set(&self, s: impl AsRef<str>) {
        let arc_str: Arc<str> = Arc::from(s.as_ref());
        let boxed = Box::new(arc_str);
        let new_ptr = Box::into_raw(boxed);
        let old_ptr = self.ptr.swap(new_ptr, Ordering::SeqCst);
        unsafe { let _ = Box::from_raw(old_ptr); }
    }

    // 获取当前字符串值
    pub fn get(&self) -> Arc<str> {
        let ptr = self.ptr.load(Ordering::SeqCst);
        unsafe { (*ptr).clone() }
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

// 允许在多线程间安全传递
unsafe impl Send for LockFreeString {}
unsafe impl Sync for LockFreeString {}

// 进度条结构体
pub struct Bar {
    pos: Arc<AtomicU64>,        // 当前进度位置
    msg: Arc<LockFreeString>,   // 进度条消息
    prefix: Arc<LockFreeString>,// 进度条后缀
    is_done: Arc<AtomicBool>,   // 是否完成标志
    thread_handle: Arc<Mutex<Option<thread::JoinHandle<()>>>>, // 渲染线程句柄
}

impl Bar {
    // 创建新的进度条
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

        // 启动渲染线程
        let handle = thread::spawn(move || {
            // 进度条动画符号
            let symbols = ["↖", "↗", "↘", "↙"];

            // 绘制进度条
            let draw = || {
                // 获取终端宽度
                let term_width = terminal_size().map(|(Width(w), _)| w).unwrap_or(80) as usize;
                // 预留空间计算
                let reserved_space = 20 + start_str.len() + end_str.len() + 10;
                let bar_length = term_width.saturating_sub(reserved_space);

                // 计算进度
                let current_pos = pos_clone.load(Ordering::Relaxed) as usize;
                let progress = (current_pos.min(count as usize)) as f64 / count.max(1) as f64;
                let filled = (progress * bar_length as f64) as usize;
                let empty = bar_length.saturating_sub(filled);

                // 动画符号索引
                let tick_idx = ((start_instant.elapsed().as_millis() / 250) % 4) as usize;
                let is_done = is_done_clone.load(Ordering::Relaxed);

                // 获取消息和后缀
                let msg_str = msg_clone.get();
                let prefix_str = prefix_clone.get();

                // 隐藏光标
                if !is_done {
                    print!("\x1b[?25l");
                }

                // 绘制进度条主体
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

                // 完成时显示光标并移动到行末
                if is_done {
                    print!("\x1b[?25h\n");
                    stdout().flush().ok();
                } else {
                    stdout().flush().ok();
                }
            };

            // 循环绘制直到完成
            loop {
                draw();
                if is_done_clone.load(Ordering::Acquire) {
                    break;
                }
                thread::sleep(Duration::from_millis(120));
            }
        });
        
        let thread_handle = Arc::new(Mutex::new(Some(handle)));

        Self {
            pos,
            msg,
            prefix,
            is_done,
            thread_handle,
        }
    }

    // 增加进度并更新消息
    pub fn grow(&self, num: u64, msg: impl Into<Cow<'static, str>>) {
        if self.is_done.load(Ordering::Relaxed) {
            return;
        }
        self.pos.fetch_add(num, Ordering::Relaxed);
        self.msg.set(msg.into().as_ref());
    }

    // 设置后缀文本
    pub fn set_suffix(&self, suffix: impl Into<Cow<'static, str>>) {
        if self.is_done.load(Ordering::Relaxed) {
            return;
        }
        self.prefix.set(suffix.into().as_ref());
    }

    // 设置消息文本
    pub fn set_message(&self, message: impl Into<Cow<'static, str>>) {
        if self.is_done.load(Ordering::Relaxed) {
            return;
        }
        self.msg.set(message.into().as_ref());
    }

    // 完成进度条并等待渲染线程结束
    pub fn done(&self) {
        self.is_done.store(true, Ordering::Release);
        if let Ok(mut handle_guard) = self.thread_handle.lock() {
            if let Some(handle) = handle_guard.take() {
                handle.join().ok();
            }
        }
        // 恢复光标显示
        print!("\x1b[?25h");
        let _ = stdout().flush();
    }
}

impl Drop for Bar {
    fn drop(&mut self) {
        self.done();
    }
}