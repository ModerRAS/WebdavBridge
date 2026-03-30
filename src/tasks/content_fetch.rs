use crate::cache::content::ContentCache;
use crate::webdav::client::UpstreamClient;
use crate::resume::RateLimiter;
use crate::webdav::types::{WebdavError, WebdavResource, RangeSpec};
use bytes::Bytes;
use std::sync::Arc;
use tokio::sync::{Mutex, watch, mpsc, oneshot};
use tracing::{info, warn, debug};

/// Request messages for ContentFetchTask
pub enum ContentFetchRequest {
    Fetch {
        path: String,
        range: Option<RangeSpec>,
        response_tx: oneshot::Sender<Result<Bytes, WebdavError>>,
    },
    GetMetadata {
        path: String,
        response_tx: oneshot::Sender<Result<WebdavResource, WebdavError>>,
    },
}

/// Single-threaded content fetch task
/// Handles ALL content fetch requests sequentially through a channel
#[derive(Clone)]
pub struct ContentFetchTask {
    upstream: Arc<UpstreamClient>,
    cache: Arc<ContentCache>,
    rate_limiter: RateLimiter,
    work_tx: Arc<Mutex<Option<mpsc::Sender<ContentFetchRequest>>>>,
    stop_tx: Arc<Mutex<Option<watch::Sender<bool>>>>,
}

impl ContentFetchTask {
    pub fn new(
        upstream: UpstreamClient,
        cache: ContentCache,
        rate_limiter: RateLimiter,
    ) -> Self {
        Self {
            upstream: Arc::new(upstream),
            cache: Arc::new(cache),
            rate_limiter,
            work_tx: Arc::new(Mutex::new(None)),
            stop_tx: Arc::new(Mutex::new(None)),
        }
    }

    /// Start the single-threaded content fetch task
    /// Returns (JoinHandle, work_tx, stop_tx)
    pub async fn start(self) -> (tokio::task::JoinHandle<()>, mpsc::Sender<ContentFetchRequest>, watch::Sender<bool>) {
        let (work_tx, work_rx) = mpsc::channel(100);
        let (stop_tx, stop_rx) = watch::channel(false);

        {
            let mut tx = self.work_tx.lock().await;
            *tx = Some(work_tx.clone());
        }
        {
            let mut tx = self.stop_tx.lock().await;
            *tx = Some(stop_tx.clone());
        }

        let this = Arc::new(self);
        let work_rx = work_rx;
        let stop_rx = stop_rx;

        let handle = tokio::spawn(async move {
            let this = this;
            this.run(work_rx, stop_rx).await;
        });

        (handle, work_tx, stop_tx)
    }

    /// Main task loop
    async fn run(self: Arc<Self>, mut work_rx: mpsc::Receiver<ContentFetchRequest>, mut stop_rx: watch::Receiver<bool>) {
        info!("Content fetch task started");

        loop {
            tokio::select! {
                _ = stop_rx.changed() => {
                    if *stop_rx.borrow() {
                        info!("Content fetch task received stop signal");
                        break;
                    }
                }
                request = work_rx.recv() => {
                    match request {
                        Some(ContentFetchRequest::Fetch { path, range, response_tx }) => {
                            let result = self.do_fetch(&path, range.as_ref()).await;
                            let _ = response_tx.send(result);
                        }
                        Some(ContentFetchRequest::GetMetadata { path, response_tx }) => {
                            let result = self.do_get_metadata(&path).await;
                            let _ = response_tx.send(result);
                        }
                        None => {
                            info!("Content fetch task received None, channel closed");
                            break;
                        }
                    }
                }
            }
        }

        info!("Content fetch task stopped");
    }

    /// Perform the actual fetch operation
    async fn do_fetch(&self, path: &str, range: Option<&RangeSpec>) -> Result<Bytes, WebdavError> {
        if let Some(range) = range {
            if self.cache.exists(path).await {
                debug!("Cache hit for range request: {}", path);
                return self.cache.read_range(path, range).await;
            }
        } else if self.cache.exists(path).await {
            let full_range = RangeSpec { start: 0, end: None };
            return self.cache.read_range(path, &full_range).await;
        }

        debug!("Cache miss, fetching from upstream: {}", path);

        if let Some(range) = range {
            self.fetch_range_from_upstream(path, range).await
        } else {
            let full_range = RangeSpec { start: 0, end: None };
            self.fetch_range_from_upstream(path, &full_range).await
        }
    }

    async fn fetch_range_from_upstream(&self, path: &str, range: &RangeSpec) -> Result<Bytes, WebdavError> {
        let bytes = self.rate_limiter.with_permit(async {
            self.upstream.get_range(path, range).await
        }).await?;

        if range.start == 0 && range.end.is_none() {
            let cache_bytes = bytes.clone();
            let stream = std::pin::pin!(futures_util::stream::once(async move {
                Ok::<Bytes, std::io::Error>(cache_bytes)
            }));
            if let Err(e) = self.cache.write_stream(path, stream).await {
                warn!("Failed to cache {}: {}", path, e);
            }
        }

        Ok(bytes)
    }

    /// Perform the actual get_metadata operation
    async fn do_get_metadata(&self, path: &str) -> Result<WebdavResource, WebdavError> {
        if let Some(resource) = self.cache.get(path).await {
            return Ok(resource);
        }

        self.rate_limiter.with_permit(async {
            self.upstream.head(path).await
        }).await
    }

    /// Fetch content - sends request through channel for single-threaded execution
    pub async fn fetch(&self, path: &str, range: Option<&RangeSpec>) -> Result<Bytes, WebdavError> {
        let (response_tx, response_rx) = oneshot::channel();

        let work_tx = {
            let tx = self.work_tx.lock().await;
            tx.clone()
        };

        if let Some(tx) = work_tx {
            tx.send(ContentFetchRequest::Fetch {
                path: path.to_string(),
                range: range.cloned(),
                response_tx,
            }).await.map_err(|e| WebdavError::UpstreamError(format!("Channel send failed: {}", e)))?;
        } else {
            return Err(WebdavError::UpstreamError("Content fetch task not started".to_string()));
        }

        response_rx.await.map_err(|e| WebdavError::UpstreamError(format!("Channel recv failed: {}", e)))?
    }

    /// Get metadata - sends request through channel for single-threaded execution
    pub async fn get_metadata(&self, path: &str) -> Result<WebdavResource, WebdavError> {
        let (response_tx, response_rx) = oneshot::channel();

        let work_tx = {
            let tx = self.work_tx.lock().await;
            tx.clone()
        };

        if let Some(tx) = work_tx {
            tx.send(ContentFetchRequest::GetMetadata {
                path: path.to_string(),
                response_tx,
            }).await.map_err(|e| WebdavError::UpstreamError(format!("Channel send failed: {}", e)))?;
        } else {
            return Err(WebdavError::UpstreamError("Content fetch task not started".to_string()));
        }

        response_rx.await.map_err(|e| WebdavError::UpstreamError(format!("Channel recv failed: {}", e)))?
    }

    pub async fn prefetch(&self, path: &str) {
        if !self.cache.exists(path).await {
            info!("Prefetching: {}", path);
            if let Err(e) = self.fetch(path, None).await {
                warn!("Prefetch failed for {}: {}", path, e);
            }
        }
    }

    pub async fn is_cached(&self, path: &str) -> bool {
        self.cache.exists(path).await
    }

    pub async fn get_cached_size(&self, path: &str) -> Option<u64> {
        self.cache.get_size(path).await.ok()
    }

    /// Signal the task to stop
    pub async fn stop(&self) {
        let stop_tx = {
            let tx = self.stop_tx.lock().await;
            tx.clone()
        };

        if let Some(tx) = stop_tx {
            let _ = tx.send(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_task_creation() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let cache = ContentCache::new(temp_dir.path().join("cache"));
    }
}