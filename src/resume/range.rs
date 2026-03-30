use crate::webdav::types::{MultiRangeSpec, RangeSpec, WebdavError, WebdavResource};
use bytes::Bytes;

/// Parse Range header value (e.g., "bytes=0-99", "bytes=100-", "bytes=-50")
pub fn parse_range_header(header: &str, total_size: u64) -> Result<RangeSpec, WebdavError> {
    // Handle "bytes=start-end", "bytes=start-", "bytes=-suffix"
    if !header.starts_with("bytes=") {
        return Err(WebdavError::InvalidPath(
            "Invalid Range header format".to_string(),
        ));
    }

    let inner = &header[6..]; // Remove "bytes="
    if inner.is_empty() {
        return Err(WebdavError::InvalidPath("Empty Range header".to_string()));
    }

    let parts: Vec<&str> = inner.splitn(2, '-').collect();
    if parts.len() != 2 {
        return Err(WebdavError::InvalidPath("Invalid Range format".to_string()));
    }

    let (start_str, end_str) = (parts[0], parts[1]);

    // bytes=-suffix (last N bytes)
    if start_str.is_empty() && !end_str.is_empty() {
        let suffix: u64 = end_str
            .parse()
            .map_err(|_| WebdavError::InvalidPath("Invalid range suffix".to_string()))?;
        if suffix == 0 {
            return Err(WebdavError::InvalidPath(
                "Zero suffix not allowed".to_string(),
            ));
        }
        let start = total_size.saturating_sub(suffix);
        return Ok(RangeSpec { start, end: None });
    }

    // bytes=start- (from start to end)
    if !start_str.is_empty() && end_str.is_empty() {
        let start: u64 = start_str
            .parse()
            .map_err(|_| WebdavError::InvalidPath("Invalid range start".to_string()))?;
        if start >= total_size {
            return Err(WebdavError::RangeNotSatisfiable {
                requested: RangeSpec { start, end: None },
                total_size,
            });
        }
        return Ok(RangeSpec { start, end: None });
    }

    // bytes=start-end (exact range)
    if !start_str.is_empty() && !end_str.is_empty() {
        let start: u64 = start_str
            .parse()
            .map_err(|_| WebdavError::InvalidPath("Invalid range start".to_string()))?;
        let end: u64 = end_str
            .parse()
            .map_err(|_| WebdavError::InvalidPath("Invalid range end".to_string()))?;

        if start > end {
            return Err(WebdavError::InvalidPath("Range start > end".to_string()));
        }

        if start >= total_size {
            return Err(WebdavError::RangeNotSatisfiable {
                requested: RangeSpec {
                    start,
                    end: Some(end),
                },
                total_size,
            });
        }

        return Ok(RangeSpec {
            start,
            end: Some(end),
        });
    }

    Err(WebdavError::InvalidPath("Invalid Range header".to_string()))
}

/// Parse a multi-range header value (e.g., "bytes=0-99,200-299,500-")
pub fn parse_range_header_multi(
    header: &str,
    total_size: u64,
) -> Result<MultiRangeSpec, WebdavError> {
    if !header.starts_with("bytes=") {
        return Err(WebdavError::InvalidPath(
            "Invalid Range header format".to_string(),
        ));
    }

    let inner = &header[6..];
    if inner.is_empty() {
        return Err(WebdavError::InvalidPath("Empty Range header".to_string()));
    }

    let range_strs: Vec<&str> = inner.split(',').collect();
    if range_strs.is_empty() {
        return Err(WebdavError::InvalidPath("No ranges specified".to_string()));
    }

    let mut ranges = Vec::new();
    for range_str in range_strs {
        let range_str = range_str.trim();
        if range_str.is_empty() {
            continue;
        }
        let spec = parse_range_header_internal(range_str, total_size)?;
        if !is_range_satisfiable(spec.start, spec.end, total_size) {
            return Err(WebdavError::RangeNotSatisfiable {
                requested: spec,
                total_size,
            });
        }
        ranges.push(spec);
    }

    if ranges.is_empty() {
        return Err(WebdavError::InvalidPath(
            "No valid ranges found".to_string(),
        ));
    }

    Ok(MultiRangeSpec { ranges, total_size })
}

fn parse_range_header_internal(header: &str, total_size: u64) -> Result<RangeSpec, WebdavError> {
    let parts: Vec<&str> = header.splitn(2, '-').collect();
    if parts.len() != 2 {
        return Err(WebdavError::InvalidPath("Invalid Range format".to_string()));
    }

    let (start_str, end_str) = (parts[0], parts[1]);

    if start_str.is_empty() && !end_str.is_empty() {
        let suffix: u64 = end_str
            .parse()
            .map_err(|_| WebdavError::InvalidPath("Invalid range suffix".to_string()))?;
        if suffix == 0 {
            return Err(WebdavError::InvalidPath(
                "Zero suffix not allowed".to_string(),
            ));
        }
        let start = total_size.saturating_sub(suffix);
        return Ok(RangeSpec { start, end: None });
    }

    if !start_str.is_empty() && end_str.is_empty() {
        let start: u64 = start_str
            .parse()
            .map_err(|_| WebdavError::InvalidPath("Invalid range start".to_string()))?;
        if start >= total_size {
            return Err(WebdavError::RangeNotSatisfiable {
                requested: RangeSpec { start, end: None },
                total_size,
            });
        }
        return Ok(RangeSpec { start, end: None });
    }

    if !start_str.is_empty() && !end_str.is_empty() {
        let start: u64 = start_str
            .parse()
            .map_err(|_| WebdavError::InvalidPath("Invalid range start".to_string()))?;
        let end: u64 = end_str
            .parse()
            .map_err(|_| WebdavError::InvalidPath("Invalid range end".to_string()))?;

        if start > end {
            return Err(WebdavError::InvalidPath("Range start > end".to_string()));
        }

        if start >= total_size {
            return Err(WebdavError::RangeNotSatisfiable {
                requested: RangeSpec {
                    start,
                    end: Some(end),
                },
                total_size,
            });
        }

        return Ok(RangeSpec {
            start,
            end: Some(end),
        });
    }

    Err(WebdavError::InvalidPath("Invalid Range header".to_string()))
}

/// Generate a random boundary string for multipart responses
fn generate_boundary() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{:x}", timestamp)
}

/// Format multipart/byteranges response body
pub fn format_multipart_ranges(
    parts: Vec<(RangeSpec, Bytes)>,
    total_size: u64,
    content_type: &str,
) -> Bytes {
    let boundary = generate_boundary();
    let mut result = Vec::new();

    for (spec, data) in parts {
        let start = spec.start;
        let end = spec.effective_end(total_size);
        let range_header = format_content_range(start, end, total_size);

        result.extend_from_slice(b"--");
        result.extend_from_slice(boundary.as_bytes());
        result.extend_from_slice(b"\r\n");
        result.extend_from_slice(b"Content-Type: ");
        result.extend_from_slice(content_type.as_bytes());
        result.extend_from_slice(b"\r\n");
        result.extend_from_slice(b"Content-Range: ");
        result.extend_from_slice(range_header.as_bytes());
        result.extend_from_slice(b"\r\n");
        result.extend_from_slice(b"\r\n");
        result.extend_from_slice(&data);
        result.extend_from_slice(b"\r\n");
    }

    result.extend_from_slice(b"--");
    result.extend_from_slice(boundary.as_bytes());
    result.extend_from_slice(b"--\r\n");

    Bytes::from(result)
}

/// Format Content-Range header value
pub fn format_content_range(start: u64, end: u64, total: u64) -> String {
    format!("bytes {}-{}/{}", start, end, total)
}

/// Check if a range is satisfiable
pub fn is_range_satisfiable(start: u64, end: Option<u64>, total_size: u64) -> bool {
    if start >= total_size {
        return false;
    }
    if let Some(e) = end {
        if e < start || e >= total_size {
            return false;
        }
    }
    true
}

/// Check if a resource supports resume (has etag or mtime)
pub fn supports_resume(resource: &WebdavResource) -> bool {
    resource.etag.is_some() || resource.modified.is_some()
}

/// Parse If-Range header (either ETag or date)
pub fn parse_if_range(if_range: &str) -> IfRange {
    let trimmed = if_range.trim();

    // ETag formats: "abc123" or W/"abc123"
    if trimmed.starts_with('"') || trimmed.starts_with("W/\"") {
        return IfRange::ETag(trimmed.to_string());
    }

    // Try to parse as HTTP-date (RFC 2822)
    // chrono::DateTime::parse_from_rfc2822 can handle these
    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(trimmed) {
        return IfRange::Date(dt.to_rfc3339());
    }

    // Fallback: treat as ETag if parsing fails
    IfRange::ETag(trimmed.to_string())
}

/// If-Range parsed value
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IfRange {
    ETag(String),
    Date(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bytes_start_end() {
        let spec = parse_range_header("bytes=0-99", 1000).unwrap();
        assert_eq!(spec.start, 0);
        assert_eq!(spec.end, Some(99));
    }

    #[test]
    fn test_parse_bytes_start_to_end() {
        let spec = parse_range_header("bytes=500-", 1000).unwrap();
        assert_eq!(spec.start, 500);
        assert_eq!(spec.end, None);
    }

    #[test]
    fn test_parse_bytes_suffix() {
        let spec = parse_range_header("bytes=-50", 1000).unwrap();
        assert_eq!(spec.start, 950);
        assert_eq!(spec.end, None);
    }

    #[test]
    fn test_parse_invalid_range() {
        let result = parse_range_header("bytes=100-50", 1000);
        assert!(result.is_err());
    }

    #[test]
    fn test_range_not_satisfiable() {
        let result = parse_range_header("bytes=1000-", 500);
        assert!(matches!(
            result,
            Err(WebdavError::RangeNotSatisfiable { .. })
        ));
    }

    #[test]
    fn test_format_content_range() {
        assert_eq!(format_content_range(0, 99, 1000), "bytes 0-99/1000");
        assert_eq!(format_content_range(500, 999, 1000), "bytes 500-999/1000");
    }

    #[test]
    fn test_is_range_satisfiable() {
        assert!(is_range_satisfiable(0, Some(99), 1000));
        assert!(is_range_satisfiable(500, None, 1000));
        assert!(!is_range_satisfiable(1000, Some(1999), 1000)); // start == total_size
        assert!(!is_range_satisfiable(100, Some(50), 1000)); // end < start
    }

    #[test]
    fn test_range_effective_end() {
        let spec = RangeSpec {
            start: 100,
            end: None,
        };
        assert_eq!(spec.effective_end(1000), 999);

        let spec2 = RangeSpec {
            start: 100,
            end: Some(200),
        };
        assert_eq!(spec2.effective_end(1000), 200);

        let spec3 = RangeSpec {
            start: 100,
            end: Some(2000),
        }; // beyond file
        assert_eq!(spec3.effective_end(1000), 999);
    }

    #[test]
    fn test_range_count() {
        let spec = RangeSpec {
            start: 0,
            end: Some(99),
        };
        assert_eq!(spec.count(1000), 100);

        let spec2 = RangeSpec {
            start: 100,
            end: None,
        };
        assert_eq!(spec2.count(1000), 900);
    }

    #[test]
    fn test_parse_empty_header() {
        let result = parse_range_header("", 1000);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_invalid_prefix() {
        let result = parse_range_header("bytes", 1000);
        assert!(result.is_err());

        let result2 = parse_range_header("invalid=0-99", 1000);
        assert!(result2.is_err());
    }

    #[test]
    fn test_parse_empty_bytes_value() {
        let result = parse_range_header("bytes=", 1000);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_zero_suffix() {
        let result = parse_range_header("bytes=-0", 1000);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_start_at_total_size() {
        // start == total_size should be RangeNotSatisfiable
        let result = parse_range_header("bytes=1000-", 1000);
        assert!(matches!(
            result,
            Err(WebdavError::RangeNotSatisfiable { .. })
        ));
    }

    #[test]
    fn test_parse_non_numeric() {
        let result = parse_range_header("bytes=abc-def", 1000);
        assert!(result.is_err());

        let result2 = parse_range_header("bytes=0-abc", 1000);
        assert!(result2.is_err());
    }

    #[test]
    fn test_parse_suffix_larger_than_file() {
        // suffix larger than file should work (returns entire file from 0)
        let spec = parse_range_header("bytes=-2000", 1000).unwrap();
        assert_eq!(spec.start, 0);
    }

    #[test]
    fn test_supports_resume() {
        let resource_with_etag =
            WebdavResource::new_file("/test".to_string(), "test".to_string(), 100)
                .with_etag("abc123".to_string());
        assert!(supports_resume(&resource_with_etag));

        let resource_with_mtime =
            WebdavResource::new_file("/test".to_string(), "test".to_string(), 100)
                .with_modified(chrono::Utc::now());
        assert!(supports_resume(&resource_with_mtime));

        let resource_without =
            WebdavResource::new_file("/test".to_string(), "test".to_string(), 100);
        assert!(!supports_resume(&resource_without));
    }

    #[test]
    fn test_parse_if_range() {
        let result = parse_if_range("\"abc123\"");
        assert!(matches!(result, IfRange::ETag(_)));

        let result2 = parse_if_range("\"xyz789\"");
        assert!(matches!(result2, IfRange::ETag(_)));
    }

    #[test]
    fn test_parse_if_range_etag() {
        // Plain ETag
        let result = parse_if_range("\"abc123\"");
        assert!(matches!(result, IfRange::ETag(etag) if etag == "\"abc123\""));

        // Weak ETag
        let result2 = parse_if_range("W/\"abc123\"");
        assert!(matches!(result2, IfRange::ETag(etag) if etag == "W/\"abc123\""));
    }

    #[test]
    fn test_parse_if_range_date() {
        // RFC 2822 date format
        let result = parse_if_range("Wed, 21 Oct 2015 07:28:00 GMT");
        assert!(matches!(result, IfRange::Date(_)));

        // Verify the date was parsed correctly
        if let IfRange::Date(date_str) = parse_if_range("Wed, 21 Oct 2015 07:28:00 GMT") {
            assert!(date_str.contains("2015-10-21"));
            assert!(date_str.contains("07:28:00"));
        }
    }

    #[test]
    fn test_is_range_satisfiable_edge_cases() {
        // start at boundary
        assert!(is_range_satisfiable(0, Some(0), 1000));
        assert!(is_range_satisfiable(999, Some(999), 1000));

        // end at boundary
        assert!(is_range_satisfiable(0, Some(999), 1000));

        // end == total_size is not satisfiable (indices are 0-based)
        assert!(!is_range_satisfiable(0, Some(1000), 1000));

        // empty range (start == end is ok for single byte)
        assert!(is_range_satisfiable(500, Some(500), 1000));
    }

    #[test]
    fn test_format_content_range_edge() {
        assert_eq!(format_content_range(0, 0, 1000), "bytes 0-0/1000");
        assert_eq!(format_content_range(999, 999, 1000), "bytes 999-999/1000");
    }

    #[test]
    fn test_parse_multi_range_two_ranges() {
        let spec = parse_range_header_multi("bytes=0-99,200-299", 1000).unwrap();
        assert_eq!(spec.ranges.len(), 2);
        assert_eq!(spec.ranges[0].start, 0);
        assert_eq!(spec.ranges[0].end, Some(99));
        assert_eq!(spec.ranges[1].start, 200);
        assert_eq!(spec.ranges[1].end, Some(299));
        assert_eq!(spec.total_size, 1000);
    }

    #[test]
    fn test_parse_multi_range_with_open_ended() {
        let spec = parse_range_header_multi("bytes=0-99,200-", 1000).unwrap();
        assert_eq!(spec.ranges.len(), 2);
        assert_eq!(spec.ranges[0].start, 0);
        assert_eq!(spec.ranges[0].end, Some(99));
        assert_eq!(spec.ranges[1].start, 200);
        assert_eq!(spec.ranges[1].end, None);
    }

    #[test]
    fn test_parse_multi_range_single_becomes_multi() {
        let spec = parse_range_header_multi("bytes=0-99", 1000).unwrap();
        assert_eq!(spec.ranges.len(), 1);
        assert_eq!(spec.ranges[0].start, 0);
        assert_eq!(spec.ranges[0].end, Some(99));
    }

    #[test]
    fn test_parse_multi_range_three_ranges() {
        let spec = parse_range_header_multi("bytes=0-99,200-299,500-", 1000).unwrap();
        assert_eq!(spec.ranges.len(), 3);
        assert_eq!(spec.ranges[2].end, None);
    }

    #[test]
    fn test_format_multipart_ranges() {
        let parts = vec![
            (
                RangeSpec {
                    start: 0,
                    end: Some(99),
                },
                Bytes::from_static(b"0123456789"),
            ),
            (
                RangeSpec {
                    start: 100,
                    end: Some(109),
                },
                Bytes::from_static(b"hello world"),
            ),
        ];
        let result = format_multipart_ranges(parts, 1000, "text/plain");
        let body = String::from_utf8(result.to_vec()).unwrap();
        assert!(body.contains("--"));
        assert!(body.contains("\r\nContent-Type: text/plain\r\n"));
        assert!(body.contains("Content-Range: bytes 0-99/1000\r\n"));
        assert!(body.contains("Content-Range: bytes 100-109/1000\r\n"));
        assert!(body.contains("\r\n\r\n"));
        assert!(body.contains("0123456789"));
        assert!(body.contains("hello world"));
    }
}
