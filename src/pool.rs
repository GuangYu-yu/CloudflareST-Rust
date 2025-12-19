use std::sync::Arc;
use std::sync::OnceLock;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// 并发限制器，使用信号量控制同时运行的任务数量
pub(crate) struct ConcurrencyLimiter {
    // 使用信号量控制并发
    semaphore: Arc<Semaphore>,
    // 最大并发数
    pub(crate) max_concurrent: usize,
}

impl ConcurrencyLimiter {
    // 创建并发限制器
    pub(crate) fn new(max_concurrent: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            max_concurrent,
        }
    }

    // 获取许可
    pub(crate) async fn acquire(&self) -> OwnedSemaphorePermit {
        self.semaphore.clone().acquire_owned().await.unwrap()
    }
}

// 全局并发限制器
pub(crate) static GLOBAL_LIMITER: OnceLock<ConcurrencyLimiter> = OnceLock::new();

// 初始化全局并发限制器
pub(crate) fn init_global_limiter(max_concurrent: usize) {
    let _ = GLOBAL_LIMITER.set(ConcurrencyLimiter::new(max_concurrent));
}

// 执行带并发限制的操作
pub(crate) async fn execute_with_rate_limit<F, Fut, T, E>(f: F) -> Result<T, E>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    // 获取许可
    let _permit = GLOBAL_LIMITER.get().unwrap().acquire().await;

    // 执行操作
    f().await
}