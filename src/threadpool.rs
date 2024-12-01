use crate::debug_log;
use std::sync::Arc;
use tokio::sync::{Semaphore, OwnedSemaphorePermit};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use std::collections::HashMap;

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
}

impl DynamicThreadPool {
    pub fn new() -> Self {
        let cpu_count = num_cpus::get();
        let initial_threads = cpu_count * 64;
        
        debug_log!("初始化线程池: CPU核心数={}, 初始线程数={}", cpu_count, initial_threads);
        
        Self {
            semaphore: Arc::new(Semaphore::new(initial_threads)),
            stats: Arc::new(Mutex::new(ThreadStats {
                threads_per_core: 64,
                stalled_tasks: 0,
                active_tasks: 0,
                last_adjust: Instant::now(),
                last_progress: Arc::new(Mutex::new(HashMap::new())),
            })),
            cpu_count,
        }
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
    }

    fn adjust_threads(&self) {
        let now = Instant::now();
        
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
            let stalled = progress_map.iter()
                .filter(|(_, last_time)| now.duration_since(**last_time) > Duration::from_secs(2))
                .count();
            (stalled, stats.active_tasks, stats.threads_per_core)
        };

        if active_tasks > 0 {
            let stall_ratio = stalled as f64 / active_tasks as f64;
            debug_log!("线程状态: 活跃任务={}, 卡顿任务={}, 卡顿比例={:.2}%, 当前每核心线程数={}", 
                active_tasks, stalled, stall_ratio * 100.0, current_threads);

            let mut stats = self.stats.lock().unwrap();
            stats.stalled_tasks = stalled;

            let new_threads_per_core = if stall_ratio > 0.2 {
                ((current_threads as f64 * 0.75) as usize).max(32)
            } else if stall_ratio < 0.05 {
                ((current_threads as f64 * 1.25) as usize)
                    .min(128)
                    .min(1024 / self.cpu_count)
            } else {
                current_threads
            };

            if new_threads_per_core != current_threads {
                let new_total = new_threads_per_core * self.cpu_count;
                let current_total = current_threads * self.cpu_count;
                
                if new_total > current_total {
                    debug_log!("增加线程数: 每核心 {} -> {}", current_threads, new_threads_per_core);
                    self.semaphore.add_permits(new_total - current_total);
                } else {
                    debug_log!("减少线程数: 每核心 {} -> {}", current_threads, new_threads_per_core);
                }
                
                let mut stats = self.stats.lock().unwrap();
                stats.threads_per_core = new_threads_per_core;
                stats.last_adjust = now;
            }
        }
    }
}

lazy_static::lazy_static! {
    pub static ref GLOBAL_POOL: DynamicThreadPool = DynamicThreadPool::new();
} 