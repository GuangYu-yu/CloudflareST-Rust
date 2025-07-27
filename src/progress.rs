use indicatif::{ProgressBar, ProgressStyle};
use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use terminal_size::{terminal_size, Width};

pub struct Bar {
    progress_bar: Arc<ProgressBar>,
    is_done: Arc<AtomicU64>,
}

impl Bar {
    pub fn new(count: u64, start_str: &str, end_str: &str) -> Self {
        // 获取终端宽度
        let term_width = terminal_size().map(|(Width(w), _)| w).unwrap_or(80) as usize;
        
        // 计算进度条长度
        // 格式: "xx/xx [进度条] 信息"
        // 预留空间: 计数器(xx/xx) + 空格 + 方括号 + 空格 + 信息预留空间
        let reserved_space = 20 + start_str.len() + end_str.len() + 10;
        let bar_length = term_width.saturating_sub(reserved_space);

        let pb = ProgressBar::new(count);
        
        let template = format!(
            "{{pos}}/{{len}} [{{bar:{}.cyan/blue}}] {} {{msg:.green}} {}", 
            bar_length,
            start_str, 
            end_str
        );
        
        pb.set_style(
            ProgressStyle::default_bar()
                .template(&template)
                .unwrap()
                .with_key("bar", move |state: &indicatif::ProgressState, w: &mut dyn std::fmt::Write| {
                    let pos = (state.pos() as f64 / state.len().unwrap_or(1) as f64 * bar_length as f64) as usize;
                    let tick_idx = (state.elapsed().as_millis() / 250) % 4;
                    write!(
                        w,
                        "{}{}{}",
                        "=".repeat(pos.saturating_sub(1)),
                        ["↖", "↗", "↘", "↙"][tick_idx as usize],
                        "_".repeat(bar_length.saturating_sub(pos)),
                    )
                    .unwrap()
                })
        );
        
        // 创建进度条并立即启用刷新
        pb.enable_steady_tick(Duration::from_millis(120));
        
        Self {
            progress_bar: Arc::new(pb),
            is_done: Arc::new(AtomicU64::new(0)),
        }
    }
    
    // 检查是否已完成
    fn is_done(&self) -> bool {
        self.is_done.load(Ordering::Relaxed) != 0
    }
    
    pub fn grow(&self, num: u64, msg: impl Into<Cow<'static, str>>) {
        // 检查是否已完成，如果已完成则不更新
        if self.is_done() {
            return;
        }
        
        self.progress_bar.set_message(msg);
        self.progress_bar.inc(num);
    }
    
    pub fn set_suffix(&self, suffix: impl Into<Cow<'static, str>>) {
        // 检查状态
        if self.is_done() {
            return;
        }
        
        self.progress_bar.set_message(suffix);
    }
    
/* 
    //完成进度条
    pub fn done(&self) {
        // 使用原子操作检查并设置完成状态
        if self.is_done.compare_exchange(0, 1, Ordering::SeqCst, Ordering::Relaxed).is_ok() {
            // 只有成功将状态从 0 改为 1 时才执行
            self.progress_bar.finish();
        }
    }
*/  
    
    /// 完成进度条但保持当前位置
    pub fn done_at_current_pos(&self) {
        // 使用原子操作检查并设置完成状态
        if self.is_done.compare_exchange(0, 1, Ordering::SeqCst, Ordering::Relaxed).is_ok() {
            // 只有成功将状态从 0 改为 1 时才执行
            self.progress_bar.as_ref().abandon();
        }
    }
}