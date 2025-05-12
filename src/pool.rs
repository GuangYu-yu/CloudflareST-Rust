use std::sync::Arc;
use tokio::sync::Semaphore;
use std::sync::OnceLock;
use crate::args::Args;

// 简化的线程池实现
pub struct ThreadPool {
    // 使用信号量控制并发
    semaphore: Arc<Semaphore>,
    // 最大线程数
    max_threads: usize,
}

impl ThreadPool {
    pub fn new() -> Self {
        let max_threads = Args::parse().max_threads as usize;
        
        Self {
            semaphore: Arc::new(Semaphore::new(max_threads)),
            max_threads,
        }
    }
    
    // 获取当前并发级别
    pub fn get_concurrency_level(&self) -> usize {
        self.max_threads
    }
    
    // 获取信号量许可
    pub async fn acquire(&self) -> tokio::sync::OwnedSemaphorePermit {
        // 获取许可
        self.semaphore.clone().acquire_owned().await.unwrap()
    }
}

// 全局线程池
pub static GLOBAL_POOL: OnceLock<ThreadPool> = OnceLock::new();

// 获取全局线程池实例
pub fn global_pool() -> &'static ThreadPool {
    GLOBAL_POOL.get_or_init(ThreadPool::new)
}

// 执行带线程池控制的操作
pub async fn execute_with_rate_limit<F, Fut, T, E>(f: F) -> Result<T, E>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    // 获取许可
    let _permit = global_pool().acquire().await;
    
    // 执行操作
    let result = f().await;
    
    // 返回结果
    result
}