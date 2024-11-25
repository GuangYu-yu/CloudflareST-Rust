use indicatif::{ProgressBar, ProgressStyle};
use std::sync::Arc;

#[derive(Clone)]
pub struct Bar {
    progress_bar: Arc<ProgressBar>,
}

impl Bar {
    pub fn new(count: u64, prefix: &str, suffix: &str) -> Self {
        // 创建进度条模板
        let template = format!(
            "{{counts}} {{bar}} {} {{msg}} {}", 
            prefix, suffix
        );

        let pb = ProgressBar::new(count);
        pb.set_style(
            ProgressStyle::default_bar()
                .template(&template)
                .unwrap()
                .progress_chars("_")
                .tick_chars("↖↗↘↙")
                .with_key("bar", |_state: &indicatif::ProgressState, w: &mut dyn std::fmt::Write| {
                    write!(w, "[{:-<width$}]", "", width = 30).unwrap()
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