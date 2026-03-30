use crate::webdav::types::{WebdavError, WebdavResource, RangeSpec};
use bytes::Bytes;
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

/// Content cache using local filesystem
pub struct ContentCache {
    cache_dir: PathBuf,
}

impl ContentCache {
    /// Create a new content cache
    pub fn new(cache_dir: impl Into<PathBuf>) -> Self {
        Self {
            cache_dir: cache_dir.into(),
        }
    }

    /// Get the full path for a cached file
    pub fn get_path(&self, relative_path: &str) -> PathBuf {
        let normalized = relative_path.trim_start_matches('/').replace('/', "\\");
        self.cache_dir.join(normalized)
    }

    /// Check if a file is cached
    pub async fn exists(&self, relative_path: &str) -> bool {
        self.get_path(relative_path).is_file()
    }

    /// Get the size of a cached file
    pub async fn get_size(&self, relative_path: &str) -> Result<u64, WebdavError> {
        let path = self.get_path(relative_path);
        let metadata = tokio::fs::metadata(&path).await?;
        Ok(metadata.len())
    }

    /// Get metadata resource - delegates to MetadataCache for actual storage
    /// ContentCache.get() returns None since ContentCache is for bytes
    pub async fn get(&self, _path: &str) -> Option<WebdavResource> {
        None
    }

    /// Read a range of bytes from a cached file
    pub async fn read_range(&self, relative_path: &str, range: &RangeSpec) -> Result<Bytes, WebdavError> {
        let path = self.get_path(relative_path);
        let mut file = tokio::fs::File::open(&path).await?;

        let file_size = file.metadata().await?.len();
        let start = range.start;
        let end = range.end.unwrap_or(file_size - 1).min(file_size - 1);

        if start >= file_size {
            return Err(WebdavError::RangeNotSatisfiable {
                requested: range.clone(),
                total_size: file_size,
            });
        }

        file.seek(tokio::io::SeekFrom::Start(start)).await?;

        let mut buffer = Vec::new();
        let mut remaining = end - start + 1;
        let mut read_buf = [0u8; 16384]; // 16KB chunks

        while remaining > 0 {
            let to_read = (remaining as usize).min(read_buf.len());
            let n = file.read(&mut read_buf[..to_read]).await?;
            if n == 0 {
                break;
            }
            buffer.extend_from_slice(&read_buf[..n]);
            remaining -= n as u64;
        }

        Ok(Bytes::from(buffer))
    }

    /// Write content from a stream to the cache
    pub async fn write_stream<S, B>(&self, relative_path: &str, stream: S) -> Result<(), WebdavError>
    where
        S: futures_util::Stream<Item = Result<B, std::io::Error>> + Unpin,
        B: bytes::Buf,
    {
        let path = self.get_path(relative_path);

        // Create parent directories
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let mut file = tokio::fs::File::create(&path).await?;

        use futures_util::StreamExt;
        let mut stream = stream;
        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result?;
            file.write_all(chunk.chunk()).await?;
        }

        file.flush().await?;
        Ok(())
    }

    /// Delete a cached file
    pub async fn delete(&self, relative_path: &str) -> Result<(), WebdavError> {
        let path = self.get_path(relative_path);
        if path.is_file() {
            tokio::fs::remove_file(&path).await?;
        }
        Ok(())
    }

    /// Ensure cache directory exists
    pub async fn ensure_dir(&self) -> Result<(), WebdavError> {
        tokio::fs::create_dir_all(&self.cache_dir).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use futures_util::stream;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_write_and_read() {
        let temp_dir = TempDir::new().unwrap();
        let cache = ContentCache::new(temp_dir.path().join("cache"));
        cache.ensure_dir().await.unwrap();

        // Write a file
        let content = Bytes::from(vec![1u8, 2, 3, 4, 5]);
        let stream = stream::iter(vec![Ok::<_, std::io::Error>(content.clone())]);
        cache.write_stream("/test.bin", stream).await.unwrap();

        // Read it back
        assert!(cache.exists("/test.bin").await);
        let size = cache.get_size("/test.bin").await.unwrap();
        assert_eq!(size, 5);
    }

    #[tokio::test]
    async fn test_read_range() {
        let temp_dir = TempDir::new().unwrap();
        let cache = ContentCache::new(temp_dir.path().join("cache"));
        cache.ensure_dir().await.unwrap();

        // Write a file with known content
        let content = Bytes::from(vec![0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        let stream = stream::iter(vec![Ok::<_, std::io::Error>(content)]);
        cache.write_stream("/range.bin", stream).await.unwrap();

        // Read range bytes=2-5
        let range = RangeSpec { start: 2, end: Some(5) };
        let result = cache.read_range("/range.bin", &range).await.unwrap();
        assert_eq!(result.as_ref(), &[2, 3, 4, 5]);
    }

    #[tokio::test]
    async fn test_delete() {
        let temp_dir = TempDir::new().unwrap();
        let cache = ContentCache::new(temp_dir.path().join("cache"));
        cache.ensure_dir().await.unwrap();

        let content = Bytes::from(vec![1u8, 2, 3]);
        let stream = stream::iter(vec![Ok::<_, std::io::Error>(content)]);
        cache.write_stream("/delete_me.bin", stream).await.unwrap();

        assert!(cache.exists("/delete_me.bin").await);
        cache.delete("/delete_me.bin").await.unwrap();
        assert!(!cache.exists("/delete_me.bin").await);
    }
}