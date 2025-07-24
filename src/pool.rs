use std::sync::Arc;
use std::sync::OnceLock;
use tokio::sync::{Semaphore, OwnedSemaphorePermit};

pub struct ThreadPool {
    // 使用信号量控制并发
    semaphore: Arc<Semaphore>,
    // 最大线程数
    pub max_threads: usize,
}

impl ThreadPool {
    // 创建线程池
    pub fn new(max_threads: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_threads)),
            max_threads,
        }
    }

    // 获取许可
    pub async fn acquire(&self) -> OwnedSemaphorePermit {
        self.semaphore.clone().acquire_owned().await.unwrap()
    }
}

// 全局线程池
pub static GLOBAL_POOL: OnceLock<ThreadPool> = OnceLock::new();

// 初始化全局线程池
pub fn init_global_pool(max_threads: usize) {
    let _ = GLOBAL_POOL.set(ThreadPool::new(max_threads));
}

// 执行带线程池控制的操作
pub async fn execute_with_rate_limit<F, Fut, T, E>(f: F) -> Result<T, E>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    // 获取许可
    let _permit = GLOBAL_POOL.get().unwrap().acquire().await;

    // 执行操作
    let result = f().await;

    // 返回结果
    result
}