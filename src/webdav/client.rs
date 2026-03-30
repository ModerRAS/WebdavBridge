use crate::config::Config;
use crate::resume::RateLimiter;
use crate::webdav::types::{WebdavError, WebdavResource, RangeSpec};
use bytes::Bytes;
use reqwest::Client;
use std::time::Duration;

/// Upstream WebDAV client with rate limiting
#[derive(Clone)]
pub struct UpstreamClient {
    client: Client,
    base_url: url::Url,
    username: Option<String>,
    password: Option<String>,
    rate_limiter: RateLimiter,
}

impl UpstreamClient {
    /// Create a new upstream client
    pub fn new(config: &Config, rate_limiter: RateLimiter) -> Result<Self, WebdavError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(WebdavError::UpstreamRequestFailed)?;
        
        let base_url = url::Url::parse(&config.upstream_url)
            .map_err(|e| WebdavError::InvalidPath(e.to_string()))?;
        
        Ok(Self {
            client,
            base_url,
            username: config.upstream_username.clone(),
            password: config.upstream_password.clone(),
            rate_limiter,
        })
    }
    
    /// Build request URL
    fn build_url(&self, path: &str) -> Result<url::Url, WebdavError> {
        let path = path.trim_start_matches('/');
        self.base_url.join(path)
            .map_err(|e| WebdavError::InvalidPath(e.to_string()))
    }
    
    /// Add authentication header
    fn add_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let (Some(u), Some(p)) = (&self.username, &self.password) {
            req.basic_auth(u, Some(p))
        } else {
            req
        }
    }
    
    /// Execute with rate limiting
    async fn with_limit<F, Fut, T>(&self, f: F) -> Result<T, WebdavError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, WebdavError>>,
    {
        self.rate_limiter.with_permit(f()).await
    }
    
    /// PROPFIND - get directory listing
    pub async fn propfind(&self, path: &str, depth: u32) -> Result<Vec<WebdavResource>, WebdavError> {
        let url = self.build_url(path)?;
        let body = r#"<?xml version="1.0" encoding="utf-8"?>
            <D:propfind xmlns:D="DAV:">
              <D:prop>
                <D:displayname/>
                <D:getcontentlength/>
                <D:getcontenttype/>
                <D:getetag/>
                <D:getlastmodified/>
                <D:resourcetype/>
              </D:prop>
            </D:propfind>"#.to_string();
        
        let depth_header = match depth {
            0 => "0",
            1 => "1",
            _ => "1", // We don't support infinity for safety
        };
        
        let result = self.with_limit(|| async {
            let req = self.client
                .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), url)
                .header("Depth", depth_header)
                .header("Content-Type", "application/xml")
                .body(body);
            
            let req = self.add_auth(req);
            let resp = req.send().await
                .map_err(WebdavError::UpstreamRequestFailed)?;
            
            if !resp.status().is_success() && resp.status().as_u16() != 207 {
                return Err(WebdavError::UpstreamError(format!(
                    "PROPFIND failed: {}", resp.status()
                )));
            }
            
            let body = resp.bytes().await
                .map_err(WebdavError::UpstreamRequestFailed)?;
            
            // Parse XML response
            Ok(self.parse_propfind_response(&body))
        }).await?;
        
        Ok(result)
    }
    
    /// Parse PROPFIND XML response
    fn parse_propfind_response(&self, body: &[u8]) -> Vec<WebdavResource> {
        let mut resources = Vec::new();
        
        // Simple XML parsing - look for href and response elements
        let xml_str = String::from_utf8_lossy(body);
        
        // Extract href values
        for line in xml_str.lines() {
            let line = line.trim();
            if line.contains("<D:href>") || line.contains("<d:href>") || line.contains("href>") {
                if let Some(href) = extract_xml_value(line, "href") {
                    let href = href.trim_start_matches('/');
                    let name = std::path::Path::new(&href)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| href.to_string());
                    
                    // Try to determine if it's a directory
                    let is_dir = line.contains("<D:collection") || 
                                 line.contains("<d:collection") ||
                                 line.contains("resourcetype><D:collection") ||
                                 line.contains("resourcetype><d:collection");
                    
                    let mut resource = if is_dir {
                        WebdavResource::new_dir(format!("/{}", href), name)
                    } else {
                        WebdavResource::new_file(format!("/{}", href), name, 0)
                    };
                    
                    // Extract size from getcontentlength
                    if let Some(size_str) = extract_xml_tag_content(&xml_str, href, "getcontentlength") {
                        if let Ok(size) = size_str.parse::<u64>() {
                            resource.size = size;
                        }
                    }
                    
                    // Extract content type
                    if let Some(ct) = extract_xml_tag_content(&xml_str, href, "getcontenttype") {
                        resource = resource.with_content_type(ct);
                    }
                    
                    // Extract etag
                    if let Some(etag) = extract_xml_tag_content(&xml_str, href, "getetag") {
                        resource = resource.with_etag(etag.trim_matches('"').to_string());
                    }
                    
                    // Extract last modified
                    if let Some(lm) = extract_xml_tag_content(&xml_str, href, "getlastmodified") {
                        if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(&lm) {
                            resource = resource.with_modified(dt.with_timezone(&chrono::Utc));
                        }
                    }
                    
                    resources.push(resource);
                }
            }
        }
        
        resources
    }
    
    /// HEAD - get metadata only
    pub async fn head(&self, path: &str) -> Result<WebdavResource, WebdavError> {
        let url = self.build_url(path)?;
        
        self.with_limit(|| async {
            let req = self.client
                .head(url)
                .header("Accept-Ranges", "bytes");
            
            let req = self.add_auth(req);
            let resp = req.send().await
                .map_err(WebdavError::UpstreamRequestFailed)?;
            
            if resp.status() == reqwest::StatusCode::NOT_FOUND {
                return Err(WebdavError::NotFound(path.to_string()));
            }
            
            if !resp.status().is_success() {
                return Err(WebdavError::UpstreamError(format!(
                    "HEAD failed: {}", resp.status()
                )));
            }
            
            let name = std::path::Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.trim_start_matches('/').to_string());
            
            let size = resp.headers()
                .get("content-length")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            
            let content_type = resp.headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            
            let etag = resp.headers()
                .get("etag")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            
            let last_modified = resp.headers()
                .get("last-modified")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| chrono::DateTime::parse_from_rfc2822(s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc));
            
            let is_dir = resp.headers()
                .get("content-type")
                .map(|v| v.to_str().unwrap_or("").contains("directory"))
                .unwrap_or(false);
            
            let mut resource = if is_dir {
                WebdavResource::new_dir(path.to_string(), name)
            } else {
                WebdavResource::new_file(path.to_string(), name, size)
            };
            
            if let Some(ct) = content_type {
                resource = resource.with_content_type(ct);
            }
            if let Some(etag) = etag {
                resource = resource.with_etag(etag);
            }
            if let Some(lm) = last_modified {
                resource = resource.with_modified(lm);
            }
            
            Ok(resource)
        }).await
    }
    
    /// GET with Range header - download file with resume support
    pub async fn get_range(&self, path: &str, range: &RangeSpec) -> Result<Bytes, WebdavError> {
        let url = self.build_url(path)?;
        
        self.with_limit(|| async {
            let req = self.client
                .get(url)
                .header("Accept-Ranges", "bytes");
            
            // Add Range header
            let range_header = if let Some(end) = range.end {
                format!("bytes={}-{}", range.start, end)
            } else {
                format!("bytes={}-", range.start)
            };
            
            let req = req.header("Range", range_header);
            let req = self.add_auth(req);
            
            let resp = req.send().await
                .map_err(WebdavError::UpstreamRequestFailed)?;
            
            let status = resp.status();
            
            // 416 Range Not Satisfiable
            if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
                return Err(WebdavError::RangeNotSatisfiable {
                    requested: range.clone(),
                    total_size: 0, // Unknown at this point
                });
            }
            
            // 200 OK means server ignored our range - return full content
            if status == reqwest::StatusCode::OK {
                let bytes = resp.bytes().await
                    .map_err(WebdavError::UpstreamRequestFailed)?;
                return Ok(bytes);
            }
            
            // 206 Partial Content - success
            if status.as_u16() == 206 {
                let bytes = resp.bytes().await
                    .map_err(WebdavError::UpstreamRequestFailed)?;
                return Ok(bytes);
            }
            
            Err(WebdavError::UpstreamError(format!(
                "GET failed: {}", status
            )))
        }).await
    }
}

/// Extract value from XML tag
fn extract_xml_value(line: &str, tag: &str) -> Option<String> {
    let start_tag = format!("<{}>", tag);
    let end_tag = format!("</{}>", tag);
    
    if let Some(start) = line.find(&start_tag) {
        let value_start = start + start_tag.len();
        if let Some(end) = line[value_start..].find(&end_tag) {
            return Some(line[value_start..value_start + end].to_string());
        }
    }
    
    // Try with namespace prefix variations
    for prefix in &["D:", "d:", "DAV:"] {
        let start_tag = format!("<{} {}>", prefix, tag.trim_start_matches(|c: char| c == 'D' || c == 'd' || c == ':'));
        let end_tag = format!("</{} {}>", prefix, tag.trim_start_matches(|c: char| c == 'D' || c == 'd' || c == ':'));
        
        if let Some(start) = line.find(&start_tag) {
            let value_start = start + start_tag.len();
            if let Some(end) = line[value_start..].find(&end_tag) {
                return Some(line[value_start..value_start + end].to_string());
            }
        }
    }
    
    None
}

/// Extract tag content across multiple lines
fn extract_xml_tag_content(xml: &str, _href: &str, tag: &str) -> Option<String> {
    let start_tag = format!("<{}>", tag);
    let end_tag = format!("</{}>", tag);
    
    let mut in_tag = false;
    for line in xml.lines() {
        if line.contains(&start_tag) {
            in_tag = true;
        }
        if in_tag {
            if let Some(value) = extract_xml_value(line, tag) {
                return Some(value);
            }
        }
        if line.contains(&end_tag) {
            in_tag = false;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use tempfile::TempDir;
    
    #[tokio::test]
    async fn test_client_creation() {
        let temp_dir = TempDir::new().unwrap();
        let config = Config {
            upstream_url: "http://localhost:8081".to_string(),
            upstream_username: None,
            upstream_password: None,
            cache_dir: temp_dir.path().join("cache"),
            metadata_db_path: temp_dir.path().join("meta.db"),
            rate_limit_permits: 2,
            metadata_update_interval_secs: 300,
            max_depth: 10,
            server_bind: "127.0.0.1:8080".to_string(),
            server_prefix: "/".to_string(),
            max_symlink_depth: 3,
        };
        
        let limiter = RateLimiter::new(2);
        let client = UpstreamClient::new(&config, limiter);
        assert!(client.is_ok());
    }
    
    #[tokio::test]
    async fn test_url_building() {
        let temp_dir = TempDir::new().unwrap();
        let config = Config {
            upstream_url: "http://localhost:8081/webdav".to_string(),
            upstream_username: None,
            upstream_password: None,
            cache_dir: temp_dir.path().join("cache"),
            metadata_db_path: temp_dir.path().join("meta.db"),
            rate_limit_permits: 2,
            metadata_update_interval_secs: 300,
            max_depth: 10,
            server_bind: "127.0.0.1:8080".to_string(),
            server_prefix: "/".to_string(),
            max_symlink_depth: 3,
        };
        
        let limiter = RateLimiter::new(2);
        let client = UpstreamClient::new(&config, limiter).unwrap();
        let url = client.build_url("/movies/video.mp4").unwrap();
        assert!(url.as_str().contains("video.mp4"));
    }
}