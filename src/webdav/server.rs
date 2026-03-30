use crate::cache::content::ContentCache;
use crate::cache::metadata::MetadataCache;
use crate::tasks::content_fetch::ContentFetchTask;
use crate::resume::range::{parse_range_header, parse_range_header_multi, format_content_range, is_range_satisfiable, format_multipart_ranges, parse_if_range, IfRange};
use crate::webdav::types::{WebdavError, WebdavResource, RangeSpec};
use bytes::Bytes;
use std::sync::Arc;
use tracing::warn;

/// WebDAV server that serves cached content to downstream clients
#[derive(Clone)]
pub struct WebdavServer {
    content_fetch: Arc<ContentFetchTask>,
    metadata_cache: Arc<MetadataCache>,
    content_cache: Option<Arc<ContentCache>>,
    max_symlink_depth: u32,
}

impl WebdavServer {
    /// Create a new WebDAV server
    pub fn new(
        content_fetch: ContentFetchTask,
        metadata_cache: MetadataCache,
    ) -> Self {
        Self {
            content_fetch: Arc::new(content_fetch),
            metadata_cache: Arc::new(metadata_cache),
            content_cache: None,
            max_symlink_depth: 3,
        }
    }

    /// Set the content cache for symlink local override writes
    pub fn with_content_cache(mut self, cache: ContentCache) -> Self {
        self.content_cache = Some(Arc::new(cache));
        self
    }

    /// Set max symlink depth
    pub fn with_max_symlink_depth(mut self, depth: u32) -> Self {
        self.max_symlink_depth = depth;
        self
    }
    
    /// Handle GET request with Range support
    pub async fn handle_get(&self, path: &str, range_header: Option<&str>, if_range: Option<&str>) -> Result<GetResponse, WebdavError> {
        // Check if this is a symlink with a local override
        if let Some(resource) = self.metadata_cache.get(path).await {
            if resource.is_symlink && resource.has_local_override {
                // Read from local content cache
                if let Some(content_cache) = &self.content_cache {
                    if content_cache.exists(path).await {
                        let full_range = RangeSpec { start: 0, end: None };
                        let bytes = content_cache.read_range(path, &full_range).await?;
                        return Ok(GetResponse {
                            bytes,
                            status: 200,
                            content_range: None,
                            etag: resource.etag,
                            content_type: resource.content_type,
                            multipart: None,
                        });
                    }
                }
            } else if resource.is_symlink {
                // Symlink without local override: resolve to upstream target
                let target = resource.symlink_target.as_deref()
                    .ok_or_else(|| WebdavError::CacheError("Symlink has no target".to_string()))?;

                // Fetch from upstream via the target path (goes through RateLimiter)
                match self.content_fetch.get_metadata(target).await {
                    Ok(_upstream_resource) => {
                        // Fetch content using the upstream target path
                        let bytes = self.content_fetch.fetch(target, None).await?;
                        return Ok(GetResponse {
                            bytes,
                            status: 200,
                            content_range: None,
                            etag: resource.etag,
                            content_type: resource.content_type,
                            multipart: None,
                        });
                    }
                    Err(WebdavError::NotFound(_)) => {
                        // Upstream target deleted, clean up the symlink
                        warn!("Symlink target {} not found, removing symlink {}", target, path);
                        let _ = self.metadata_cache.delete(path).await;
                        return Err(WebdavError::NotFound(path.to_string()));
                    }
                    Err(e) => return Err(e),
                }
            }
        }

        // Regular (non-symlink) GET handling
        let resource = match self.content_fetch.get_metadata(path).await {
            Ok(r) => r,
            Err(WebdavError::NotFound(_)) => {
                return Err(WebdavError::NotFound(path.to_string()));
            }
            Err(e) => return Err(e),
        };
        
        let allow_range = match (if_range, &resource.etag, &resource.modified) {
            (Some(if_range_str), Some(etag), _) => {
                let parsed = parse_if_range(if_range_str);
                match parsed {
                    IfRange::ETag(if_etag) => if_etag == *etag,
                    IfRange::Date(date_str) => {
                        if let Some(modified) = &resource.modified {
                            modified.to_rfc3339() == date_str
                        } else {
                            false
                        }
                    }
                }
            }
            (Some(if_range_str), None, Some(modified)) => {
                let parsed = parse_if_range(if_range_str);
                match parsed {
                    IfRange::Date(date_str) => modified.to_rfc3339() == date_str,
                    IfRange::ETag(_) => false,
                }
            }
            (Some(_), None, None) => false,
            (None, _, _) => true,
        };
        
        let range_header = range_header.unwrap_or("");
        if range_header.is_empty() || range_header.to_lowercase() == "bytes" {
            let bytes = self.content_fetch.fetch(path, None).await?;
            return Ok(GetResponse {
                bytes,
                status: 200,
                content_range: None,
                etag: resource.etag,
                content_type: resource.content_type,
                multipart: None,
            });
        }
        
        if !allow_range {
            let bytes = self.content_fetch.fetch(path, None).await?;
            return Ok(GetResponse {
                bytes,
                status: 200,
                content_range: None,
                etag: resource.etag,
                content_type: resource.content_type,
                multipart: None,
            });
        }
        
        let has_comma = range_header.contains(',');
        if has_comma {
            return self.handle_multi_range(path, &resource, range_header).await;
        }
        
        let range_spec = match parse_range_header(range_header, resource.size) {
            Ok(r) => r,
            Err(WebdavError::RangeNotSatisfiable { .. }) => {
                return Err(WebdavError::RangeNotSatisfiable {
                    requested: RangeSpec { start: 0, end: None },
                    total_size: resource.size,
                });
            }
            Err(e) => return Err(e),
        };
        
        if !is_range_satisfiable(range_spec.start, range_spec.end, resource.size) {
            return Err(WebdavError::RangeNotSatisfiable {
                requested: range_spec.clone(),
                total_size: resource.size,
            });
        }
        
        let bytes = self.content_fetch.fetch(path, Some(&range_spec)).await?;
        let end = range_spec.effective_end(resource.size);
        
        Ok(GetResponse {
            bytes,
            status: 206,
            content_range: Some(format_content_range(range_spec.start, end, resource.size)),
            etag: resource.etag,
            content_type: resource.content_type,
            multipart: None,
        })
    }
    
    async fn handle_multi_range(&self, path: &str, resource: &WebdavResource, range_header: &str) -> Result<GetResponse, WebdavError> {
        let multi_spec = match parse_range_header_multi(range_header, resource.size) {
            Ok(r) => r,
            Err(WebdavError::RangeNotSatisfiable { .. }) => {
                return Err(WebdavError::RangeNotSatisfiable {
                    requested: RangeSpec { start: 0, end: None },
                    total_size: resource.size,
                });
            }
            Err(e) => return Err(e),
        };
        
        let mut parts = Vec::new();
        for spec in &multi_spec.ranges {
            let bytes = self.content_fetch.fetch(path, Some(spec)).await?;
            parts.push((spec.clone(), bytes));
        }
        
        let content_type = resource.content_type.clone().unwrap_or_else(|| "application/octet-stream".to_string());
        let body = format_multipart_ranges(parts, resource.size, &content_type);
        
        Ok(GetResponse {
            bytes: body,
            status: 206,
            content_range: None,
            etag: resource.etag.clone(),
            content_type: Some(format!("multipart/byteranges; boundary={}", "BOUNDARY")),
            multipart: Some(true),
        })
    }
    
    /// Handle HEAD request
    pub async fn handle_head(&self, path: &str) -> Result<HeadResponse, WebdavError> {
        let resource = self.content_fetch.get_metadata(path).await?;
        Ok(HeadResponse {
            size: resource.size,
            etag: resource.etag,
            content_type: resource.content_type,
            supports_range: resource.supports_resume,
        })
    }
    
    /// Handle PROPFIND request - list directory contents
    pub async fn handle_propfind(&self, path: &str) -> Result<Vec<WebdavResource>, WebdavError> {
        // Try to get from metadata cache first
        let resources = if path == "/" || path.is_empty() {
            self.metadata_cache.get_children("/").await
        } else {
            self.metadata_cache.get_children(path).await
        };
        
        if resources.is_empty() {
            // Try to fetch from upstream
            warn!("No cached metadata for {}, upstream fetch not implemented in server", path);
            return Ok(Vec::new());
        }
        
        Ok(resources)
    }

    /// Handle COPY request - create a symlink at the destination pointing to the same upstream target
    pub async fn handle_copy(&self, src_path: &str, dest_path: &str, overwrite: bool) -> Result<u16, WebdavError> {
        // Look up source resource
        let src_resource = self.metadata_cache.get(src_path).await
            .ok_or_else(|| WebdavError::NotFound(src_path.to_string()))?;

        // Check if destination already exists
        let dest_exists = self.metadata_cache.get(dest_path).await.is_some();
        if dest_exists && !overwrite {
            return Err(WebdavError::PreconditionFailed(
                format!("Destination {} already exists and Overwrite is false", dest_path),
            ));
        }

        // Determine the upstream target
        let target = if src_resource.is_symlink {
            src_resource.symlink_target.clone().unwrap_or_else(|| src_path.to_string())
        } else {
            src_path.to_string()
        };

        // Check for cycles
        if self.metadata_cache.would_create_cycle(dest_path, &target, self.max_symlink_depth).await {
            return Err(WebdavError::SymlinkCycle(
                format!("Creating symlink {} -> {} would create a cycle", dest_path, target),
            ));
        }

        if src_resource.is_dir {
            // Directory copy: recursively create symlinks for all children
            self.copy_directory_symlinks(src_path, dest_path, &src_resource).await?;
        } else {
            // File copy: create a single symlink
            let name = dest_path.rsplit('/').next().unwrap_or(dest_path).to_string();
            let symlink = WebdavResource::new_symlink(
                dest_path.to_string(),
                name,
                target,
                false,
                src_resource.size,
            )
            .with_content_type_opt(src_resource.content_type.clone());
            self.metadata_cache.put(&symlink).await?;
        }

        Ok(if dest_exists { 204 } else { 201 })
    }

    /// Recursively create symlinks for a directory COPY
    async fn copy_directory_symlinks(&self, src_path: &str, dest_path: &str, src_resource: &WebdavResource) -> Result<(), WebdavError> {
        // Create the directory symlink entry
        let dir_name = dest_path.rsplit('/').next().unwrap_or(dest_path).to_string();
        let dir_target = if src_resource.is_symlink {
            src_resource.symlink_target.clone().unwrap_or_else(|| src_path.to_string())
        } else {
            src_path.to_string()
        };
        let dir_symlink = WebdavResource::new_symlink(
            dest_path.to_string(),
            dir_name,
            dir_target,
            true,
            0,
        );
        self.metadata_cache.put(&dir_symlink).await?;

        // Recursively copy children
        let children = self.metadata_cache.get_children(src_path).await;
        for child in children {
            let child_rel = child.path.strip_prefix(src_path).unwrap_or(&child.path);
            let child_dest = format!("{}{}", dest_path, child_rel);

            if child.is_dir {
                Box::pin(self.copy_directory_symlinks(&child.path, &child_dest, &child)).await?;
            } else {
                let target = if child.is_symlink {
                    child.symlink_target.clone().unwrap_or_else(|| child.path.clone())
                } else {
                    child.path.clone()
                };
                let name = child_dest.rsplit('/').next().unwrap_or(&child_dest).to_string();
                let symlink = WebdavResource::new_symlink(
                    child_dest,
                    name,
                    target,
                    false,
                    child.size,
                )
                .with_content_type_opt(child.content_type.clone());
                self.metadata_cache.put(&symlink).await?;
            }
        }

        Ok(())
    }

    /// Handle MOVE request - move a symlink to a new path (symlink_target unchanged)
    pub async fn handle_move(&self, src_path: &str, dest_path: &str, overwrite: bool) -> Result<u16, WebdavError> {
        // Look up source resource
        let src_resource = self.metadata_cache.get(src_path).await
            .ok_or_else(|| WebdavError::NotFound(src_path.to_string()))?;

        // Only symlinks can be moved
        if !src_resource.is_symlink {
            return Err(WebdavError::Forbidden(
                "Cannot move non-symlink resources".to_string(),
            ));
        }

        // Check if destination already exists
        let dest_exists = self.metadata_cache.get(dest_path).await.is_some();
        if dest_exists && !overwrite {
            return Err(WebdavError::PreconditionFailed(
                format!("Destination {} already exists and Overwrite is false", dest_path),
            ));
        }

        if src_resource.is_dir {
            // Directory move: recursively move all children
            self.move_directory_symlinks(src_path, dest_path).await?;
        }

        // Move the resource itself: create at new path, delete from old
        let name = dest_path.rsplit('/').next().unwrap_or(dest_path).to_string();
        let mut moved = src_resource.clone();
        moved.path = dest_path.to_string();
        moved.name = name;
        self.metadata_cache.put(&moved).await?;
        self.metadata_cache.delete(src_path).await?;

        Ok(if dest_exists { 204 } else { 201 })
    }

    /// Recursively move children for a directory MOVE
    async fn move_directory_symlinks(&self, src_path: &str, dest_path: &str) -> Result<(), WebdavError> {
        let children = self.metadata_cache.get_children(src_path).await;
        for child in children {
            let child_rel = child.path.strip_prefix(src_path).unwrap_or(&child.path);
            let child_dest = format!("{}{}", dest_path, child_rel);

            if child.is_dir {
                Box::pin(self.move_directory_symlinks(&child.path, &child_dest)).await?;
            }

            let name = child_dest.rsplit('/').next().unwrap_or(&child_dest).to_string();
            let mut moved = child.clone();
            moved.path = child_dest;
            moved.name = name;
            self.metadata_cache.put(&moved).await?;
            self.metadata_cache.delete(&child.path).await?;
        }

        Ok(())
    }

    /// Handle PUT request - write content to a symlink's local override
    pub async fn handle_put(&self, path: &str, body: Bytes) -> Result<u16, WebdavError> {
        let resource = self.metadata_cache.get(path).await
            .ok_or_else(|| WebdavError::NotFound(path.to_string()))?;

        if !resource.is_symlink {
            return Err(WebdavError::Forbidden(
                "Cannot write to non-symlink resources".to_string(),
            ));
        }

        let content_cache = self.content_cache.as_ref()
            .ok_or_else(|| WebdavError::CacheError("Content cache not configured".to_string()))?;

        // Write content to local cache
        let stream = futures_util::stream::once(async move {
            Ok::<Bytes, std::io::Error>(body)
        });
        let stream = std::pin::pin!(stream);
        content_cache.write_stream(path, stream).await?;

        // Mark as having local override
        self.metadata_cache.set_local_override(path, true).await?;

        Ok(204)
    }

    /// Handle DELETE request - delete a symlink
    pub async fn handle_delete(&self, path: &str) -> Result<u16, WebdavError> {
        let resource = self.metadata_cache.get(path).await
            .ok_or_else(|| WebdavError::NotFound(path.to_string()))?;

        if !resource.is_symlink {
            return Err(WebdavError::Forbidden(
                "Cannot delete non-symlink resources".to_string(),
            ));
        }

        // If there's a local override, delete it from content cache
        if resource.has_local_override {
            if let Some(content_cache) = &self.content_cache {
                content_cache.delete(path).await?;
            }
        }

        // If it's a directory, recursively delete children
        if resource.is_dir {
            self.delete_directory_recursive(path).await?;
        }

        // Delete from metadata cache
        self.metadata_cache.delete(path).await?;

        Ok(204)
    }

    /// Recursively delete children of a directory
    async fn delete_directory_recursive(&self, dir_path: &str) -> Result<(), WebdavError> {
        let children = self.metadata_cache.get_children(dir_path).await;
        for child in children {
            if child.is_dir {
                Box::pin(self.delete_directory_recursive(&child.path)).await?;
            }
            if child.has_local_override {
                if let Some(content_cache) = &self.content_cache {
                    content_cache.delete(&child.path).await?;
                }
            }
            self.metadata_cache.delete(&child.path).await?;
        }
        Ok(())
    }

    /// Get the metadata cache (for external access)
    pub fn metadata_cache(&self) -> &Arc<MetadataCache> {
        &self.metadata_cache
    }
}

/// Response for GET
pub struct GetResponse {
    pub bytes: Bytes,
    pub status: u16,
    pub content_range: Option<String>,
    pub etag: Option<String>,
    pub content_type: Option<String>,
    pub multipart: Option<bool>,
}

/// Response for HEAD
pub struct HeadResponse {
    pub size: u64,
    pub etag: Option<String>,
    pub content_type: Option<String>,
    pub supports_range: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webdav::types::WebdavResource;
    use chrono::Utc;

    #[test]
    fn test_get_response_fields() {
        let response = GetResponse {
            bytes: Bytes::from_static(b"test content"),
            status: 200,
            content_range: None,
            etag: Some("abc123".to_string()),
            content_type: Some("text/plain".to_string()),
            multipart: None,
        };
        
        assert_eq!(response.status, 200);
        assert_eq!(response.bytes.len(), 12);
        assert_eq!(response.etag, Some("abc123".to_string()));
    }

    #[test]
    fn test_head_response_fields() {
        let response = HeadResponse {
            size: 1024,
            etag: Some("def456".to_string()),
            content_type: Some("video/mp4".to_string()),
            supports_range: true,
        };
        
        assert_eq!(response.size, 1024);
        assert!(response.supports_range);
    }

    #[test]
    fn test_range_spec_effective_end() {
        let spec = RangeSpec { start: 0, end: Some(99) };
        assert_eq!(spec.effective_end(1000), 99);
        
        let spec2 = RangeSpec { start: 500, end: None };
        assert_eq!(spec2.effective_end(1000), 999);
    }

    #[tokio::test]
    async fn test_webdav_resource_builder() {
        let resource = WebdavResource::new_file("/test.mp4".to_string(), "test.mp4".to_string(), 1024)
            .with_etag("etag123".to_string())
            .with_content_type("video/mp4".to_string());
        
        assert_eq!(resource.path, "/test.mp4");
        assert_eq!(resource.size, 1024);
        assert!(resource.etag.is_some());
        assert!(resource.content_type.is_some());
        assert!(resource.supports_resume);
    }
}