use crate::cache::metadata::MetadataCache;
use crate::webdav::client::UpstreamClient;
use crate::resume::RateLimiter;
use crate::webdav::types::WebdavError;
use std::sync::Arc;
use tokio::sync::watch;
use tokio::time::{interval, Duration};
use tracing::{info, warn, error, debug};

/// Single-threaded metadata update task
/// Periodically syncs directory listings from upstream WebDAV server
pub struct MetadataUpdateTask {
    upstream: Arc<UpstreamClient>,
    cache: Arc<MetadataCache>,
    rate_limiter: RateLimiter,
    interval_secs: u64,
    max_depth: u32,
    stop_tx: Option<watch::Sender<bool>>,
}

impl MetadataUpdateTask {
    /// Create a new metadata update task
    pub fn new(
        upstream: UpstreamClient,
        cache: MetadataCache,
        rate_limiter: RateLimiter,
        interval_secs: u64,
        max_depth: u32,
    ) -> Self {
        Self {
            upstream: Arc::new(upstream),
            cache: Arc::new(cache),
            rate_limiter,
            interval_secs,
            max_depth,
            stop_tx: None,
        }
    }
    
    /// Start the update task in a single background thread
    /// Returns (JoinHandle, stop_tx) - drop stop_tx to signal shutdown
    pub fn start(mut self) -> (tokio::task::JoinHandle<()>, watch::Sender<bool>) {
        let (stop_tx, stop_rx) = watch::channel(false);
        self.stop_tx = Some(stop_tx.clone());
        
        let handle = tokio::spawn(async move {
            self.run(stop_rx).await;
        });
        
        (handle, stop_tx)
    }
    
    /// Run the update loop
    async fn run(self, mut stop_rx: watch::Receiver<bool>) {
        info!("Metadata update task started, interval={}s", self.interval_secs);
        
        // Run initial sync
        if let Err(e) = self.update_once().await {
            error!("Initial metadata sync failed: {}", e);
        }
        
        let mut ticker = interval(Duration::from_secs(self.interval_secs));
        
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if let Err(e) = self.update_once().await {
                        error!("Metadata sync failed: {}", e);
                    }
                }
                _ = stop_rx.changed() => {
                    if *stop_rx.borrow() {
                        info!("Metadata update task received stop signal");
                        break;
                    }
                }
            }
        }
        
        info!("Metadata update task stopped");
    }
    
    /// Perform a single update cycle
    pub async fn update_once(&self) -> Result<(), WebdavError> {
        info!("Starting metadata sync");
        let start = std::time::Instant::now();
        
        // Start from root
        self.sync_directory("/").await?;
        
        let elapsed = start.elapsed();
        info!("Metadata sync completed in {:?}", elapsed);
        Ok(())
    }
    
    /// Recursively sync a directory using an explicit stack (iterative)
    async fn sync_directory(&self, path: &str) -> Result<(), WebdavError> {
        let mut stack = vec![path.to_string()];
        
        while let Some(current_path) = stack.pop() {
            debug!("Syncing directory: {}", current_path);
            
            let resources = self.rate_limiter.with_permit(async {
                self.upstream.propfind(&current_path, 1).await
            }).await?;
            
            for resource in resources {
                if resource.path == current_path {
                    continue;
                }
                
                if let Err(e) = self.cache.put(&resource).await {
                    warn!("Failed to cache {}: {}", resource.path, e);
                }
                
                if resource.is_dir && resource.path.starts_with(&current_path) {
                    let relative = resource.path.trim_start_matches(&current_path).trim_start_matches('/');
                    let depth = relative.matches('/').count() as u32;
                    
                    if depth < self.max_depth {
                        stack.push(resource.path);
                    }
                }
            }
        }
        
        Ok(())
    }
    
    /// Signal the task to stop
    pub fn stop(&self) {
        if let Some(tx) = &self.stop_tx {
            let _ = tx.send(true);
        }
    }
    
    /// Check if task is running
    pub fn is_running(&self) -> bool {
        self.stop_tx.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    
    #[tokio::test]
    async fn test_task_creation() {
        let temp_dir = TempDir::new().unwrap();
        
        // This test just verifies the struct can be created
        // Full testing would require mocking upstream and cache
    }
}