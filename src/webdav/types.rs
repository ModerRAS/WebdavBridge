use chrono::{DateTime, Utc};

/// Represents a WebDAV resource (file or directory) with cached metadata
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WebdavResource {
    /// Full path on the upstream server (e.g., "/movies/video.mp4")
    pub path: String,

    /// Display name (filename or directory name)
    pub name: String,

    /// MIME type (e.g., "video/mp4", "audio/mpeg")
    pub content_type: Option<String>,

    /// File size in bytes
    pub size: u64,

    /// ETag for cache validation
    pub etag: Option<String>,

    /// Last modified timestamp
    pub modified: Option<DateTime<Utc>>,

    /// True if this is a directory
    pub is_dir: bool,

    /// True if this resource supports resume (has etag or mtime)
    pub supports_resume: bool,

    /// True if this resource is a symlink to an upstream path
    #[serde(default)]
    pub is_symlink: bool,

    /// The upstream path this symlink points to (None for regular resources)
    #[serde(default)]
    pub symlink_target: Option<String>,

    /// True if the symlink has a local override (written via PUT)
    #[serde(default)]
    pub has_local_override: bool,
}

impl WebdavResource {
    /// Create a new file resource
    pub fn new_file(path: String, name: String, size: u64) -> Self {
        Self {
            path,
            name,
            content_type: None,
            size,
            etag: None,
            modified: None,
            is_dir: false,
            supports_resume: false,
            is_symlink: false,
            symlink_target: None,
            has_local_override: false,
        }
    }

    /// Create a new directory resource
    pub fn new_dir(path: String, name: String) -> Self {
        Self {
            path,
            name,
            content_type: None,
            size: 0,
            etag: None,
            modified: None,
            is_dir: true,
            supports_resume: false,
            is_symlink: false,
            symlink_target: None,
            has_local_override: false,
        }
    }

    /// Create a new symlink resource pointing to an upstream path
    pub fn new_symlink(path: String, name: String, target: String, is_dir: bool, size: u64) -> Self {
        Self {
            path,
            name,
            content_type: None,
            size,
            etag: None,
            modified: None,
            is_dir,
            supports_resume: false,
            is_symlink: true,
            symlink_target: Some(target),
            has_local_override: false,
        }
    }

    /// Set content type from extension
    pub fn with_content_type(mut self, ct: String) -> Self {
        self.content_type = Some(ct);
        self
    }

    /// Set content type from optional value
    pub fn with_content_type_opt(mut self, ct: Option<String>) -> Self {
        self.content_type = ct;
        self
    }

    /// Set ETag
    pub fn with_etag(mut self, etag: String) -> Self {
        self.etag = Some(etag);
        self.supports_resume = true;
        self
    }

    /// Set modified time
    pub fn with_modified(mut self, modified: DateTime<Utc>) -> Self {
        self.modified = Some(modified);
        self.supports_resume = true;
        self
    }
}

/// Range request specification
#[derive(Debug, Clone)]
pub struct RangeSpec {
    pub start: u64,
    pub end: Option<u64>, // None means "to end of file"
}

/// Multiple range specs from a single Range header
#[derive(Debug, Clone)]
pub struct MultiRangeSpec {
    pub ranges: Vec<RangeSpec>,
    pub total_size: u64,
}

impl RangeSpec {
    /// Parse from "bytes=start-end" format
    pub fn parse(s: &str, total_size: u64) -> Option<Self> {
        // Handle "bytes=start-end", "bytes=start-", "bytes=-suffix"
        if !s.starts_with("bytes=") {
            return None;
        }

        let inner = &s[6..];
        let parts: Vec<&str> = inner.split('-').collect();

        if parts.len() != 2 {
            return None;
        }

        if parts[0].is_empty() && parts[1].is_empty() {
            return None;
        }

        if parts[0].is_empty() {
            // bytes=-suffix: last N bytes
            let suffix: u64 = parts[1].parse().ok()?;
            let start = total_size.saturating_sub(suffix);
            return Some(RangeSpec { start, end: None });
        }

        if parts[1].is_empty() {
            // bytes=start-: from start to end
            let start: u64 = parts[0].parse().ok()?;
            return Some(RangeSpec { start, end: None });
        }

        // bytes=start-end
        let start: u64 = parts[0].parse().ok()?;
        let end: u64 = parts[1].parse().ok()?;

        if start > end {
            return None;
        }

        Some(RangeSpec {
            start,
            end: Some(end),
        })
    }

    /// Get the effective end position
    pub fn effective_end(&self, total_size: u64) -> u64 {
        self.end
            .unwrap_or(total_size.saturating_sub(1))
            .min(total_size.saturating_sub(1))
    }

    /// Get the count of bytes in this range
    pub fn count(&self, total_size: u64) -> u64 {
        let end = self.effective_end(total_size);
        end.saturating_sub(self.start) + 1
    }
}

/// Cache status for a resource lookup
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheStatus {
    /// Resource found in cache and is fresh
    Hit,
    /// Resource not found in cache
    Miss,
    /// Resource found but is stale (needs refresh)
    Stale,
}

/// Error types for WebDAV operations
#[derive(Debug, thiserror::Error)]
pub enum WebdavError {
    #[error("Upstream error: {0}")]
    UpstreamError(String),

    #[error("Upstream request failed: {0}")]
    UpstreamRequestFailed(#[from] reqwest::Error),

    #[error("Resource not found: {0}")]
    NotFound(String),

    #[error("Range not satisfiable: requested {requested:?} but total size is {total_size}")]
    RangeNotSatisfiable {
        requested: RangeSpec,
        total_size: u64,
    },

    #[error("Cache error: {0}")]
    CacheError(String),

    #[error("Invalid path: {0}")]
    InvalidPath(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Symlink cycle detected: {0}")]
    SymlinkCycle(String),

    #[error("Symlink depth exceeded: max depth is {max_depth}")]
    SymlinkDepthExceeded { max_depth: u32 },

    #[error("Precondition failed: {0}")]
    PreconditionFailed(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),
}

pub type Result<T> = std::result::Result<T, WebdavError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_range_parse_standard_range() {
        let spec = RangeSpec::parse("bytes=0-499", 1000).unwrap();
        assert_eq!(spec.start, 0);
        assert_eq!(spec.end, Some(499));
    }

    #[test]
    fn test_range_parse_open_ended() {
        let spec = RangeSpec::parse("bytes=500-", 1000).unwrap();
        assert_eq!(spec.start, 500);
        assert_eq!(spec.end, None);
    }

    #[test]
    fn test_range_parse_suffix() {
        let spec = RangeSpec::parse("bytes=-500", 1000).unwrap();
        assert_eq!(spec.start, 500);
        assert_eq!(spec.end, None);
    }

    #[test]
    fn test_range_parse_suffix_full_file() {
        let spec = RangeSpec::parse("bytes=-1000", 1000).unwrap();
        assert_eq!(spec.start, 0);
        assert_eq!(spec.end, None);
    }

    #[test]
    fn test_range_parse_invalid_prefix() {
        assert!(RangeSpec::parse("bytes", 1000).is_none());
        assert!(RangeSpec::parse("invalid", 1000).is_none());
        assert!(RangeSpec::parse("bytes=", 1000).is_none());
    }

    #[test]
    fn test_range_parse_invalid_range() {
        // start > end
        assert!(RangeSpec::parse("bytes=500-400", 1000).is_none());
    }

    #[test]
    fn test_range_parse_empty_parts() {
        // both empty
        assert!(RangeSpec::parse("bytes=-", 1000).is_none());
    }

    #[test]
    fn test_range_parse_non_numeric() {
        assert!(RangeSpec::parse("bytes=abc-def", 1000).is_none());
        assert!(RangeSpec::parse("bytes=0-abc", 1000).is_none());
    }

    #[test]
    fn test_range_effective_end() {
        let spec = RangeSpec {
            start: 100,
            end: Some(199),
        };
        assert_eq!(spec.effective_end(1000), 199);

        // open-ended
        let spec = RangeSpec {
            start: 900,
            end: None,
        };
        assert_eq!(spec.effective_end(1000), 999);
    }

    #[test]
    fn test_range_effective_end_exceeds_size() {
        let spec = RangeSpec {
            start: 0,
            end: Some(1500),
        };
        assert_eq!(spec.effective_end(1000), 999);
    }

    #[test]
    fn test_range_count() {
        let spec = RangeSpec {
            start: 0,
            end: Some(499),
        };
        assert_eq!(spec.count(1000), 500);

        let spec = RangeSpec {
            start: 0,
            end: None,
        };
        assert_eq!(spec.count(1000), 1000);

        let spec = RangeSpec {
            start: 500,
            end: None,
        };
        assert_eq!(spec.count(1000), 500);
    }

    #[test]
    fn test_webdav_resource_new_file() {
        let resource =
            WebdavResource::new_file("/movies/test.mp4".to_string(), "test.mp4".to_string(), 1024);
        assert_eq!(resource.path, "/movies/test.mp4");
        assert_eq!(resource.name, "test.mp4");
        assert_eq!(resource.size, 1024);
        assert!(!resource.is_dir);
        assert!(!resource.supports_resume);
    }

    #[test]
    fn test_webdav_resource_new_dir() {
        let resource = WebdavResource::new_dir("/movies".to_string(), "movies".to_string());
        assert_eq!(resource.path, "/movies");
        assert_eq!(resource.name, "movies");
        assert_eq!(resource.size, 0);
        assert!(resource.is_dir);
        assert!(!resource.supports_resume);
    }

    #[test]
    fn test_webdav_resource_builder_with_etag() {
        let resource =
            WebdavResource::new_file("/movies/test.mp4".to_string(), "test.mp4".to_string(), 1024)
                .with_etag("abc123".to_string())
                .with_content_type("video/mp4".to_string());
        assert_eq!(resource.etag, Some("abc123".to_string()));
        assert_eq!(resource.content_type, Some("video/mp4".to_string()));
        assert!(resource.supports_resume);
    }

    #[test]
    fn test_webdav_resource_builder_with_modified() {
        let resource =
            WebdavResource::new_file("/movies/test.mp4".to_string(), "test.mp4".to_string(), 1024)
                .with_modified(Utc::now());
        assert!(resource.modified.is_some());
        assert!(resource.supports_resume);
    }

    #[test]
    fn test_webdav_resource_new_symlink() {
        let resource = WebdavResource::new_symlink(
            "/local/link.mp4".to_string(),
            "link.mp4".to_string(),
            "/upstream/test.mp4".to_string(),
            false,
            1024,
        );
        assert_eq!(resource.path, "/local/link.mp4");
        assert_eq!(resource.name, "link.mp4");
        assert!(resource.is_symlink);
        assert_eq!(resource.symlink_target, Some("/upstream/test.mp4".to_string()));
        assert!(!resource.has_local_override);
        assert!(!resource.is_dir);
        assert_eq!(resource.size, 1024);
    }

    #[test]
    fn test_webdav_resource_symlink_dir() {
        let resource = WebdavResource::new_symlink(
            "/local/movies".to_string(),
            "movies".to_string(),
            "/upstream/movies".to_string(),
            true,
            0,
        );
        assert!(resource.is_symlink);
        assert!(resource.is_dir);
        assert_eq!(resource.symlink_target, Some("/upstream/movies".to_string()));
    }

    #[test]
    fn test_webdav_resource_symlink_with_content_type() {
        let resource = WebdavResource::new_symlink(
            "/local/link.mp4".to_string(),
            "link.mp4".to_string(),
            "/upstream/test.mp4".to_string(),
            false,
            1024,
        )
        .with_content_type("video/mp4".to_string());
        assert_eq!(resource.content_type, Some("video/mp4".to_string()));
    }

    #[test]
    fn test_webdav_resource_symlink_serialization() {
        let resource = WebdavResource::new_symlink(
            "/local/link.mp4".to_string(),
            "link.mp4".to_string(),
            "/upstream/test.mp4".to_string(),
            false,
            1024,
        );

        let json = serde_json::to_string(&resource).unwrap();
        let deserialized: WebdavResource = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.is_symlink, true);
        assert_eq!(deserialized.symlink_target, Some("/upstream/test.mp4".to_string()));
        assert_eq!(deserialized.has_local_override, false);
    }

    #[test]
    fn test_webdav_resource_normal_file_defaults() {
        // Normal files should have symlink fields at default values
        let resource = WebdavResource::new_file("/test.txt".to_string(), "test.txt".to_string(), 100);
        assert!(!resource.is_symlink);
        assert_eq!(resource.symlink_target, None);
        assert!(!resource.has_local_override);

        // Verify serialization/deserialization of normal files keeps defaults
        let json = serde_json::to_string(&resource).unwrap();
        let deserialized: WebdavResource = serde_json::from_str(&json).unwrap();
        assert!(!deserialized.is_symlink);
        assert_eq!(deserialized.symlink_target, None);
        assert!(!deserialized.has_local_override);
    }

    #[test]
    fn test_backward_compat_deserialization() {
        // Simulate old JSON without symlink fields (backward compatibility)
        let old_json = r#"{"path":"/test.txt","name":"test.txt","content_type":null,"size":100,"etag":null,"modified":null,"is_dir":false,"supports_resume":false}"#;
        let resource: WebdavResource = serde_json::from_str(old_json).unwrap();
        assert!(!resource.is_symlink);
        assert_eq!(resource.symlink_target, None);
        assert!(!resource.has_local_override);
    }

    #[test]
    fn test_with_content_type_opt() {
        let resource = WebdavResource::new_file("/test.mp4".to_string(), "test.mp4".to_string(), 100)
            .with_content_type_opt(Some("video/mp4".to_string()));
        assert_eq!(resource.content_type, Some("video/mp4".to_string()));

        let resource2 = WebdavResource::new_file("/test.mp4".to_string(), "test.mp4".to_string(), 100)
            .with_content_type_opt(None);
        assert_eq!(resource2.content_type, None);
    }
}
