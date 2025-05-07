use std::sync::Arc;
use tokio::sync::Semaphore;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use num_cpus;
use std::sync::OnceLock;
use std::mem::ManuallyDrop;
use crate::args::Args;

// 简化的线程池实现
pub struct ThreadPool {
    // 使用信号量控制并发
    semaphore: Arc<Semaphore>,
    // 活跃任务数
    active_tasks: Arc<AtomicUsize>,
    // 保留一些统计信息
    stats: Arc<Mutex<PoolStats>>,
    // CPU核心数
    cpu_count: usize,
    // 当前允许的最大许可数
    max_permits: Arc<AtomicUsize>,
    // 当前活跃许可计数
    current_permits: Arc<AtomicUsize>,
    // 最大线程数
    max_threads: usize,
}

struct PoolStats {
    // 每核心线程数
    threads_per_core: usize,
    // 上次调整时间
    last_adjust: Instant,
    // 总任务执行时间统计
    total_duration: f64,
    // CPU计算时间统计
    cpu_duration: f64,
    p90_cpu_duration: f64,
    ewma_factor: f64,
    // 最近一次调整的方向 (1: 增加, -1: 减少, 0: 不变)
    last_adjustment_direction: i8,
    // 连续相同方向调整的次数
    consecutive_adjustments: usize,
}

pub struct CustomPermit {
    permit: ManuallyDrop<tokio::sync::OwnedSemaphorePermit>,
    max_permits: Arc<AtomicUsize>,
    current_permits: Arc<AtomicUsize>,
    dropped: bool,
}

impl Drop for CustomPermit {
    fn drop(&mut self) {
        if self.dropped {
            return;
        }
        
        // 获取当前许可计数和最大许可数
        let current = self.current_permits.load(Ordering::SeqCst);
        let max = self.max_permits.load(Ordering::SeqCst);
        
        // 减少当前活跃许可计数
        self.current_permits.fetch_sub(1, Ordering::SeqCst);
        
        // 如果当前许可数超过最大许可数，则不返回到信号量池中
        if current > max {
            // 标记为已处理，避免重复处理
            self.dropped = true;
            return;
        }
        
        // 正常返回许可
        unsafe {
            ManuallyDrop::drop(&mut self.permit);
        }
        self.dropped = true;
    }
}

pub struct CpuTimer<'a> {
    start: Instant,
    paused: Option<Instant>,
    total_paused: Duration,
    pool: &'a ThreadPool,
}

impl<'a> CpuTimer<'a> {
    // 记录暂停时的累计时间
    pub fn pause(&mut self) {
        if self.paused.is_none() {
            self.paused = Some(Instant::now());
        }
    }

    // 恢复计时
    pub fn resume(&mut self) {
        if let Some(pause_time) = self.paused.take() {
            self.total_paused += pause_time.elapsed();
        }
    }

    // 结束计时
    pub fn finish(self) {
        let elapsed = if let Some(pause_time) = self.paused {
            pause_time - self.start - self.total_paused
        } else {
            self.start.elapsed() - self.total_paused
        };
        self.pool.record_cpu_duration(elapsed);
    }
}

impl ThreadPool {
    pub fn new() -> Self {
        let cpu_count = num_cpus::get();
        let max_threads = Args::parse().max_threads as usize;
        // 初始每核心64个线程
        let initial_threads_per_core = 64;
        let initial_threads = cpu_count * initial_threads_per_core;
        
        let pool = Self {
            semaphore: Arc::new(Semaphore::new(initial_threads)),
            active_tasks: Arc::new(AtomicUsize::new(0)),
            max_permits: Arc::new(AtomicUsize::new(initial_threads)),
            current_permits: Arc::new(AtomicUsize::new(0)),
            stats: Arc::new(Mutex::new(PoolStats {
                threads_per_core: initial_threads_per_core,
                last_adjust: Instant::now(),
                total_duration: 0.0,
                cpu_duration: 0.0,
                p90_cpu_duration: 0.0,
                ewma_factor: 0.1,
                last_adjustment_direction: 0,
                consecutive_adjustments: 0,
            })),
            cpu_count,
            max_threads,
        };
        
        // 启动后台调整任务
        let pool_clone = pool.clone();
        tokio::spawn(async move {
            // 线程数调整间隔
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                pool_clone.adjust_threads();
            }
        });
        
        pool
    }
    
    // 克隆线程池
    fn clone(&self) -> Self {
        Self {
            semaphore: self.semaphore.clone(),
            active_tasks: self.active_tasks.clone(),
            stats: self.stats.clone(),
            cpu_count: self.cpu_count,
            max_permits: self.max_permits.clone(),
            current_permits: self.current_permits.clone(),
            max_threads: self.max_threads,
        }
    }
    
    // 获取当前并发级别
    pub fn get_concurrency_level(&self) -> usize {
        let stats = self.stats.lock().unwrap();
        self.cpu_count * stats.threads_per_core
    }
    
    // 开始任务
    pub fn start_task(&self) {
        self.active_tasks.fetch_add(1, Ordering::SeqCst);
    }
    
    // 结束任务
    pub fn end_task(&self) {
        self.active_tasks.fetch_sub(1, Ordering::SeqCst);
    }
    
    // 记录总任务执行时间
    pub fn record_task_duration(&self, duration: Duration) {
        let duration_ms = duration.as_secs_f64() * 1000.0;
        let mut stats = self.stats.lock().unwrap();
        
        // 使用EWMA更新总任务时间统计
        if stats.total_duration == 0.0 {
            // 首次初始化
            stats.total_duration = duration_ms;
        } else {
            // 更新统计值
            stats.total_duration = stats.total_duration * (1.0 - stats.ewma_factor) + 
                                 duration_ms * stats.ewma_factor;
        }
    }
    
    // 记录CPU计算时间
    pub fn record_cpu_duration(&self, duration: Duration) {
        let duration_ms = duration.as_secs_f64() * 1000.0;
        let mut stats = self.stats.lock().unwrap();
        
        // 使用EWMA更新CPU时间统计
        if stats.cpu_duration == 0.0 {
            // 首次初始化
            stats.cpu_duration = duration_ms;
            stats.p90_cpu_duration = duration_ms;
        } else {
            // 更新统计值
            stats.cpu_duration = stats.cpu_duration * (1.0 - stats.ewma_factor) + 
                               duration_ms * stats.ewma_factor;
            
            // 如果当前值大于P90，则更新P90
            if duration_ms > stats.p90_cpu_duration {
                stats.p90_cpu_duration = stats.p90_cpu_duration * (1.0 - stats.ewma_factor) + 
                                    duration_ms * stats.ewma_factor;
            } else {
                // 缓慢降低P90
                stats.p90_cpu_duration = stats.p90_cpu_duration * (1.0 - stats.ewma_factor * 0.1);
            }
        }
    }
    
    // 开始CPU计时
    pub fn start_cpu_timer(&self) -> CpuTimer {
        CpuTimer {
            start: Instant::now(),
            paused: None,
            total_paused: Duration::default(),
            pool: self,
        }
    }
    
    // 动态调整线程数（基于CPU时间）
    pub fn adjust_threads(&self) {
        let now = Instant::now();
        
        // 获取当前活跃任务数
        let active_tasks = self.active_tasks.load(Ordering::SeqCst);
        if active_tasks == 0 {
            return;
        }
        
        // 获取必要的统计信息，但尽快释放锁
        let (current_threads_per_core, cpu_duration, p90_cpu_duration) = {
            let stats = self.stats.lock().unwrap();
            (
                stats.threads_per_core,
                stats.cpu_duration,
                stats.p90_cpu_duration
            )
        };
        
        // 计算负载因子 (活跃任务数 / 总线程数)
        let total_threads = self.cpu_count * current_threads_per_core;
        let load_factor = active_tasks as f64 / total_threads as f64;
        
        // 分析CPU计算时间
        let mut adjustment_factor = 1.0;
        
        if cpu_duration > 0.0 {
            // 计算CPU时间波动比
            let cpu_ratio = p90_cpu_duration / cpu_duration;
            
            // 根据CPU时间和负载因子综合调整
            let min_factor = 0.6;
            let max_factor = 1.2;
            
            let cpu_weight = (cpu_ratio - 1.0).min(100.0).max(0.0);
            adjustment_factor = min_factor + (max_factor - min_factor) * 
                ((1.0 - load_factor) * 0.4 + cpu_weight * 0.6);
        }
        
        // 计算新的每核心线程数
        let min_threads_per_core = 5; // 绝对最小线程数
        let mut new_threads_per_core = ((current_threads_per_core as f64 * adjustment_factor) as usize)
            .max(min_threads_per_core) // 最小线程数下限
            .min(self.max_threads / self.cpu_count); // 总线程数上限
        
        // 防止频繁小幅度调整
        if (new_threads_per_core as f64 / current_threads_per_core as f64 - 1.0).abs() < 0.1 {
            new_threads_per_core = current_threads_per_core;
        }
        
        // 只有当线程数需要变化时才再次获取锁
        if new_threads_per_core != current_threads_per_core {
            let mut stats = self.stats.lock().unwrap();
            
            // 再次检查，避免在获取锁期间其他线程已经调整过
            if new_threads_per_core != stats.threads_per_core {
                let new_total = new_threads_per_core * self.cpu_count;
                let current_total = stats.threads_per_core * self.cpu_count;
                
                // 确定调整方向
                let direction = if new_threads_per_core > stats.threads_per_core { 1 } else { -1 };
                
                // 检查是否连续同向调整
                if direction == stats.last_adjustment_direction {
                    stats.consecutive_adjustments += 1;
                } else {
                    stats.consecutive_adjustments = 1;
                }
                
                // 如果连续同向调整次数过多，增加调整幅度
                if stats.consecutive_adjustments > 3 {
                    if direction > 0 {
                        new_threads_per_core = (new_threads_per_core * 12 / 10)
                            .min(self.max_threads / self.cpu_count); // 总线程数上限
                    } else {
                        new_threads_per_core = (new_threads_per_core * 8 / 10).max(min_threads_per_core);
                    }
                }
                
                stats.last_adjustment_direction = direction;
                
                if new_total > current_total {
                    // 增加线程数
                    self.semaphore.add_permits(new_total - current_total);
                    self.max_permits.store(new_total, Ordering::SeqCst);
                } else if new_total < current_total {
                    // 更新最大许可数，新的许可将受此限制
                    self.max_permits.store(new_total, Ordering::SeqCst);
                }
                
                stats.threads_per_core = new_threads_per_core;
                stats.last_adjust = now;
            }
        }
    }
    
    // 获取信号量许可
    pub async fn acquire(&self) -> CustomPermit {
        // 获取许可
        let permit = self.semaphore.clone().acquire_owned().await.unwrap();
        
        // 更新当前活跃许可计数
        self.current_permits.fetch_add(1, Ordering::SeqCst);
        
        // 返回自定义许可包装
        CustomPermit {
            permit: ManuallyDrop::new(permit),
            max_permits: self.max_permits.clone(),
            current_permits: self.current_permits.clone(),
            dropped: false,
        }
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
    // 开始任务（内部会处理活跃任务计数）
    let _permit = global_pool().acquire().await;
    
    // 记录开始时间
    let start_time = Instant::now();
    
    // 执行操作
    let result = f().await;
    
    // 记录总任务执行时间
    global_pool().record_task_duration(start_time.elapsed());
    
    // 结束任务（CustomPermit的Drop实现会自动处理）
    result
}