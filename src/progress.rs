use indicatif::{ProgressBar, ProgressStyle};
use std::sync::Arc;
use std::time::Duration;
use terminal_size::{terminal_size, Width};

pub struct Bar {
    progress_bar: Arc<ProgressBar>,
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
        
        // 启用稳定的刷新间隔
        pb.enable_steady_tick(Duration::from_millis(120));

        Self {
            progress_bar: Arc::new(pb),
        }
    }
    
    pub fn grow(&self, num: u64, msg: String) {
        self.progress_bar.set_message(msg);
        self.progress_bar.inc(num);
    }
    
    pub fn set_suffix(&self, suffix: String) {
        self.progress_bar.set_message(suffix);
    }
    
    pub fn done(&self) {
        // 使用 finish() ，进度条保留在屏幕上
        self.progress_bar.finish();
    }
}

impl Drop for Bar {
    fn drop(&mut self) {
        self.done();
    }
}