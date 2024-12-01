#[cfg(feature = "debug")]
use tracing_subscriber::{fmt, EnvFilter};

pub fn init_debug() {
    #[cfg(feature = "debug")]
    {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| {
            fmt()
                .with_env_filter(EnvFilter::from_default_env()
                    .add_directive(tracing::Level::DEBUG.into()))
                .with_target(false)
                .with_thread_ids(true)
                .with_thread_names(true)
                .with_file(true)
                .with_line_number(true)
                .init();
        });
    }
}

// debug宏,只在debug feature启用时生效
#[cfg(feature = "debug")]
#[macro_export]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        tracing::debug!($($arg)*);
    }
}

#[cfg(not(feature = "debug"))]
#[macro_export]
macro_rules! debug_log {
    ($($arg:tt)*) => {};
} 