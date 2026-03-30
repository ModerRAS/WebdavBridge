use crate::webdav::types::{WebdavError, WebdavResource};
use sled::{Config, Db, Tree};
use std::path::Path;

/// Metadata cache using sled embedded database
#[derive(Clone)]
pub struct MetadataCache {
    db: Db,
    tree: Tree,
}

impl MetadataCache {
    /// Open or create a metadata cache database
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, WebdavError> {
        let config = Config::new()
            .path(path)
            .temporary(false);
        let db = config.open().map_err(|e| WebdavError::CacheError(e.to_string()))?;
        let tree = db.open_tree(b"metadata")
            .map_err(|e| WebdavError::CacheError(e.to_string()))?;
        Ok(Self { db, tree })
    }
    
    /// Get a resource from cache
    pub async fn get(&self, path: &str) -> Option<WebdavResource> {
        self.tree
            .get(path.as_bytes())
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_slice(&v).ok())
    }
    
    /// Put a resource into cache
    pub async fn put(&self, resource: &WebdavResource) -> Result<(), WebdavError> {
        let key = resource.path.as_bytes();
        let value = serde_json::to_vec(resource)
            .map_err(|e| WebdavError::SerializationError(e.to_string()))?;
        self.tree
            .insert(key, value)
            .map_err(|e| WebdavError::CacheError(e.to_string()))?;
        Ok(())
    }
    
    /// Delete a resource from cache
    pub async fn delete(&self, path: &str) -> Result<(), WebdavError> {
        self.tree
            .remove(path.as_bytes())
            .map_err(|e| WebdavError::CacheError(e.to_string()))?;
        Ok(())
    }
    
    /// Get all resources in a directory
    pub async fn get_children(&self, dir_path: &str) -> Vec<WebdavResource> {
        let prefix = if dir_path == "/" { "" } else { dir_path };
        let mut results = Vec::new();
        
        for item in self.tree.iter() {
            if let Ok((key, value)) = item {
                let path = String::from_utf8_lossy(&key);
                let is_child = if prefix.is_empty() {
                    path.len() > 1 && path.starts_with('/') && !path[1..].contains('/')
                } else {
                    path.starts_with(prefix) && 
                    path.len() > prefix.len() && 
                    path[prefix.len()..].starts_with('/') &&
                    !path[prefix.len() + 1..].contains('/')
                };
                
                if is_child {
                    if let Ok(resource) = serde_json::from_slice::<WebdavResource>(&value) {
                        results.push(resource);
                    }
                }
            }
        }
        results
    }
    
    /// Iterate over all cached resources
    pub async fn iter_all(&self) -> impl Iterator<Item = WebdavResource> + '_ {
        self.tree.iter().filter_map(|item| {
            item.ok().and_then(|(_, v)| serde_json::from_slice(&v).ok())
        })
    }
    
    /// Clear all cached metadata
    pub async fn clear(&self) -> Result<(), WebdavError> {
        self.tree.clear()
            .map_err(|e| WebdavError::CacheError(e.to_string()))?;
        Ok(())
    }

    /// Check if a path is a symlink
    pub async fn is_symlink(&self, path: &str) -> bool {
        self.get(path).await.map(|r| r.is_symlink).unwrap_or(false)
    }

    /// Get the symlink target for a path
    pub async fn get_symlink_target(&self, path: &str) -> Option<String> {
        self.get(path).await.and_then(|r| {
            if r.is_symlink {
                r.symlink_target
            } else {
                None
            }
        })
    }

    /// Set the local override flag for a symlink
    pub async fn set_local_override(&self, path: &str, has_override: bool) -> Result<(), WebdavError> {
        if let Some(mut resource) = self.get(path).await {
            resource.has_local_override = has_override;
            self.put(&resource).await?;
        }
        Ok(())
    }

    /// Check if a symlink has a local override
    pub async fn has_local_override(&self, path: &str) -> bool {
        self.get(path).await.map(|r| r.has_local_override).unwrap_or(false)
    }

    /// Find all symlinks pointing to a specific upstream target
    pub async fn get_by_target(&self, target: &str) -> Vec<WebdavResource> {
        let mut results = Vec::new();
        for item in self.tree.iter() {
            if let Ok((_, value)) = item {
                if let Ok(resource) = serde_json::from_slice::<WebdavResource>(&value) {
                    if resource.is_symlink && resource.symlink_target.as_deref() == Some(target) {
                        results.push(resource);
                    }
                }
            }
        }
        results
    }

    /// Delete all symlinks pointing to a specific upstream target (cascade delete)
    pub async fn delete_by_target(&self, target: &str) -> Result<Vec<String>, WebdavError> {
        let symlinks = self.get_by_target(target).await;
        let mut deleted_paths = Vec::new();
        for symlink in &symlinks {
            self.delete(&symlink.path).await?;
            deleted_paths.push(symlink.path.clone());
        }
        Ok(deleted_paths)
    }

    /// Check for symlink cycles: would creating a symlink from `path` to `target` create a cycle?
    pub async fn would_create_cycle(&self, path: &str, target: &str, max_depth: u32) -> bool {
        // A cycle exists if following symlinks from `target` leads back to `path`
        let mut current = target.to_string();
        let mut depth = 0;
        while depth < max_depth {
            if current == path {
                return true;
            }
            match self.get_symlink_target(&current).await {
                Some(next_target) => {
                    current = next_target;
                    depth += 1;
                }
                None => return false,
            }
        }
        // If we exceeded max depth, treat as problematic
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    
    #[tokio::test]
    async fn test_put_get() {
        let temp_dir = TempDir::new().unwrap();
        let cache = MetadataCache::open(temp_dir.path().join("test.db")).await.unwrap();
        
        let resource = WebdavResource::new_file("/test.txt".to_string(), "test.txt".to_string(), 100);
        cache.put(&resource).await.unwrap();
        
        let retrieved = cache.get("/test.txt").await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().path, "/test.txt");
    }
    
    #[tokio::test]
    async fn test_delete() {
        let temp_dir = TempDir::new().unwrap();
        let cache = MetadataCache::open(temp_dir.path().join("test.db")).await.unwrap();
        
        let resource = WebdavResource::new_file("/test.txt".to_string(), "test.txt".to_string(), 100);
        cache.put(&resource).await.unwrap();
        
        cache.delete("/test.txt").await.unwrap();
        assert!(cache.get("/test.txt").await.is_none());
    }
    
    #[tokio::test]
    async fn test_get_children() {
        let temp_dir = TempDir::new().unwrap();
        let cache = MetadataCache::open(temp_dir.path().join("test.db")).await.unwrap();
        
        // Add parent dir
        cache.put(&WebdavResource::new_dir("/movies".to_string(), "movies".to_string())).await.unwrap();
        // Add children
        cache.put(&WebdavResource::new_file("/movies/video1.mp4".to_string(), "video1.mp4".to_string(), 1000)).await.unwrap();
        cache.put(&WebdavResource::new_file("/movies/video2.mp4".to_string(), "video2.mp4".to_string(), 2000)).await.unwrap();
        
        let children = cache.get_children("/movies").await;
        assert_eq!(children.len(), 2);
    }
}