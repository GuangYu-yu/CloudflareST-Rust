use std::sync::Arc;
use std::sync::OnceLock;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

struct ConcurrencyLimiter {
    semaphore: Arc<Semaphore>,
    max_concurrent: usize,
}

impl ConcurrencyLimiter {
    fn new(max_concurrent: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            max_concurrent,
        }
    }
}

static GLOBAL_LIMITER: OnceLock<ConcurrencyLimiter> = OnceLock::new();

pub(crate) fn init_global_limiter(max_concurrent: usize) {
    let _ = GLOBAL_LIMITER.set(ConcurrencyLimiter::new(max_concurrent));
}

pub(crate) async fn acquire_permit() -> OwnedSemaphorePermit {
    GLOBAL_LIMITER.get().unwrap().semaphore.clone().acquire_owned().await.unwrap()
}

pub(crate) fn max_concurrent() -> usize {
    GLOBAL_LIMITER.get().unwrap().max_concurrent
}