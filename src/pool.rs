use std::sync::Arc;
use tokio::sync::{Semaphore, OwnedSemaphorePermit};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use std::collections::{HashMap, VecDeque};
use num_cpus;

pub struct DynamicThreadPool {
    semaphore: Arc<Semaphore>,
    stats: Arc<Mutex<ThreadStats>>,
    cpu_count: usize,
}

struct ThreadStats {
    threads_per_core: usize,
    stalled_tasks: usize,
    active_tasks: usize,
    last_adjust: Instant,
    last_progress: Arc<Mutex<HashMap<usize, Instant>>>,
    // 添加可用ID队列
    available_ids: VecDeque<usize>,
    next_id: usize, // 下一个未使用的ID
}

impl DynamicThreadPool {
    pub fn new() -> Self {
        let cpu_count = num_cpus::get();
        let initial_threads = cpu_count * 64;
        
        // 创建一个空的可用ID队列
        let available_ids = VecDeque::new();
        
        Self {
            semaphore: Arc::new(Semaphore::new(initial_threads)),
            stats: Arc::new(Mutex::new(ThreadStats {
                threads_per_core: 64,
                stalled_tasks: 0,
                active_tasks: 0,
                last_adjust: Instant::now(),
                last_progress: Arc::new(Mutex::new(HashMap::new())),
                available_ids,
                next_id: 1, // 从1开始分配ID
            })),
            cpu_count,
        }
    }

    // 添加获取当前并发级别的方法
    pub fn get_concurrency_level(&self) -> usize {
        let stats = self.stats.lock().unwrap();
        self.cpu_count * stats.threads_per_core
    }

    // 获取任务ID的新方法
    pub fn get_task_id(&self) -> usize {
        let mut stats = self.stats.lock().unwrap();
        
        // 优先从可用ID队列中获取
        if let Some(id) = stats.available_ids.pop_front() {
            return id;
        }
        
        // 如果没有可用ID，则分配新ID
        let id = stats.next_id;
        stats.next_id += 1;
        
        // 确保ID不超过一个较大的值
        if stats.next_id > 10_000 {
            // 如果所有ID都已分配，则从1重新开始
            // 这种情况应该很少发生，因为我们会回收ID
            stats.next_id = 1;
        }
        
        id
    }

    pub async fn acquire(&self) -> OwnedSemaphorePermit {
        self.adjust_threads();
        self.semaphore.clone().acquire_owned().await.unwrap()
    }

    pub fn record_progress(&self, task_id: usize) {
        let stats = self.stats.lock().unwrap();
        stats.last_progress.lock().unwrap().insert(task_id, Instant::now());
    }

    pub fn start_task(&self, task_id: usize) {
        let mut stats = self.stats.lock().unwrap();
        stats.active_tasks += 1;
        stats.last_progress.lock().unwrap().insert(task_id, Instant::now());
    }

    pub fn end_task(&self, task_id: usize) {
        let mut stats = self.stats.lock().unwrap();
        stats.active_tasks -= 1;
        stats.last_progress.lock().unwrap().remove(&task_id);
        
        // 回收任务ID
        stats.available_ids.push_back(task_id);
    }

    fn adjust_threads(&self) {
        let now = Instant::now();
        
        // 增加初始等待时间
        if now.duration_since(self.stats.lock().unwrap().last_adjust) < Duration::from_secs(5) {
            return;
        }

        let adjust_needed = {
            let stats = self.stats.lock().unwrap();
            now.duration_since(stats.last_adjust) >= Duration::from_secs(1)
        };

        if !adjust_needed {
            return;
        }

        let (stalled, active_tasks, current_threads) = {
            let stats = self.stats.lock().unwrap();
            let progress_map = stats.last_progress.lock().unwrap();
            
            // 超过3秒没有心跳的任务视为卡住
            let stalled = progress_map.iter()
                .filter(|(_, last_time)| now.duration_since(**last_time) > Duration::from_secs(3))
                .count();
                
            (stalled, stats.active_tasks, stats.threads_per_core)
        };

        // 只有当有活跃任务时才进行调整
        if active_tasks > 0 {
            let stall_ratio = stalled as f64 / active_tasks as f64;
            let mut stats = self.stats.lock().unwrap();
            stats.stalled_tasks = stalled;

            // 根据卡顿比例调整因子
            let min_factor = 0.6;
            let max_factor = 1.2;
            let adjustment_factor = min_factor + (max_factor - min_factor) * (1.0 - stall_ratio);

            // 计算新的每核心线程数
            let new_threads_per_core = ((current_threads as f64 * adjustment_factor) as usize)
                .max(32)  // 最小每核心32个线程
                .min(256) // 最大每核心256个线程
                .min(1024 / self.cpu_count); // 总线程数不超过1024

            // 只有当线程数需要变化时才进行调整
            if new_threads_per_core != current_threads {
                let new_total = new_threads_per_core * self.cpu_count;
                let current_total = current_threads * self.cpu_count;
                
                if new_total > current_total {
                    // 增加线程数
                    self.semaphore.add_permits(new_total - current_total);
                } else if new_total < current_total {
                    // 注意：Semaphore不支持直接减少permits，但会随着任务完成自然减少
                }
                
                stats.threads_per_core = new_threads_per_core;
                stats.last_adjust = now;
            }
        }
    }
}

// 全局线程池
lazy_static::lazy_static! {
    pub static ref GLOBAL_POOL: DynamicThreadPool = DynamicThreadPool::new();
}

/// 执行带线程池控制的操作
pub async fn execute_with_rate_limit<F, Fut, T, E>(f: F) -> Result<T, E>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    // 使用池分配的任务ID，而不是随机生成
    let task_id = GLOBAL_POOL.get_task_id();
    
    // 开始任务
    GLOBAL_POOL.start_task(task_id);
    
    // 获取线程许可
    let _permit = GLOBAL_POOL.acquire().await;
    
    // 记录进度
    GLOBAL_POOL.record_progress(task_id);
    
    // 执行操作
    let result = f().await;
    
    // 结束任务
    GLOBAL_POOL.end_task(task_id);
    
    result
}