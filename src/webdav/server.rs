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
        }
    }
    
    /// Handle GET request with Range support
    pub async fn handle_get(&self, path: &str, range_header: Option<&str>, if_range: Option<&str>) -> Result<GetResponse, WebdavError> {
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