use std::sync::Arc;
use tokio::sync::Semaphore;

pub mod range;

pub use range::{
    parse_range_header, format_content_range, is_range_satisfiable,
    supports_resume, parse_if_range, IfRange,
};

/// Rate limiter using semaphore for upstream access
/// Limits concurrent connections to upstream server to a fixed number (typically 2)
#[derive(Clone)]
pub struct RateLimiter {
    semaphore: Arc<Semaphore>,
    permits: usize,
}

impl RateLimiter {
    /// Create a new rate limiter with specified permit count
    pub fn new(permits: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(permits)),
            permits,
        }
    }
    
    /// Get the number of permits (max concurrent connections)
    pub fn permits(&self) -> usize {
        self.permits
    }
    
    /// Acquire a permit, waiting if none available
    pub async fn acquire(&self) -> RateLimiterPermit<'_> {
        RateLimiterPermit {
            permit: self.semaphore.acquire().await.unwrap(),
        }
    }
    
    /// Execute a future with a permit from this rate limiter
    pub async fn with_permit<F, T>(&self, future: F) -> T
    where
        F: std::future::Future<Output = T>,
    {
        let _permit = self.acquire().await;
        future.await
    }
}

/// RAII guard for an acquired rate limiter permit
pub struct RateLimiterPermit<'a> {
    permit: tokio::sync::SemaphorePermit<'a>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    
    #[tokio::test]
    async fn test_rate_limiter_basic() {
        let limiter = RateLimiter::new(2);
        assert_eq!(limiter.permits(), 2);
    }
    
    #[tokio::test]
    async fn test_acquire_release() {
        let limiter = RateLimiter::new(1);
        
        // First acquire should succeed
        let permit1 = limiter.acquire().await;
        drop(permit1);
        
        // Second acquire should also succeed after first is dropped
        let permit2 = limiter.acquire().await;
        drop(permit2);
    }
    
    #[tokio::test]
    async fn test_concurrent_limit() {
        let limiter = RateLimiter::new(2);
        let counter = Arc::new(AtomicUsize::new(0));
        let mut handles = vec![];
        
        for _ in 0..5 {
            let limiter = limiter.clone();
            let counter = counter.clone();
            handles.push(tokio::spawn(async move {
                let _permit = limiter.acquire().await;
                let before = counter.fetch_add(1, Ordering::SeqCst);
                assert!(before < 2, "More than 2 concurrent accesses!");
                tokio::task::yield_now().await;
                counter.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        
        for handle in handles {
            handle.await.unwrap();
        }
        
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }
    
    #[tokio::test]
    async fn test_with_permit() {
        let limiter = RateLimiter::new(1);
        let result = limiter.with_permit(async {
            42
        }).await;
        assert_eq!(result, 42);
    }
}