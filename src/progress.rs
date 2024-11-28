use indicatif::{ProgressBar, ProgressStyle};
use std::sync::Arc;
use terminal_size::{terminal_size, Width};

#[derive(Clone, Debug)]
pub struct Bar {
    progress_bar: Arc<ProgressBar>,
}

impl Bar {
    pub fn new(count: u64, prefix: &str, suffix: &str) -> Self {
        // 获取终端宽度
        let term_width = terminal_size().map(|(Width(w), _)| w).unwrap_or(80) as usize;
        
        // 计算进度条长度
        // 格式: "xx/xx [进度条] 信息"
        // 预留空间: 计数器(xx/xx) + 空格 + 方括号 + 空格 + 信息预留空间
        let reserved_space = 20 + prefix.len() + 20;  // 预留20字符给信息显示
        let bar_length = term_width.saturating_sub(reserved_space);

        // 创建进度条模板
        let template = format!(
            "{{pos}}/{{len}} {{bar}} {} {{msg}} {}", 
            prefix, suffix
        );

        let pb = ProgressBar::new(count);
        pb.set_style(
            ProgressStyle::default_bar()
                .template(&template)
                .unwrap()
                .progress_chars("=-")
                .tick_chars("↖↗↘↙")
                .with_key("bar", move |state: &indicatif::ProgressState, w: &mut dyn std::fmt::Write| {
                    let pos = (state.pos() as f64 / state.len().unwrap_or(1) as f64 * bar_length as f64) as usize;
                    let tick_idx = (state.elapsed().as_millis() / 250) % 4;
                    write!(
                        w,
                        "[{}{}{}]",
                        "=".repeat(pos.saturating_sub(1)),
                        ["↖", "↗", "↘", "↙"][tick_idx as usize],
                        "-".repeat(bar_length.saturating_sub(pos)),
                    )
                    .unwrap()
                })
        );

        Self {
            progress_bar: Arc::new(pb),
        }
    }

    pub fn grow(&self, num: u64, msg: &str) {
        self.progress_bar.set_message(msg.to_string());
        self.progress_bar.inc(num);
    }

    pub fn done(&self) {
        self.progress_bar.finish_and_clear();
    }
}

impl Drop for Bar {
    fn drop(&mut self) {
        self.done();
    }
} 