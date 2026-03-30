//! WebdavBridge - WebDAV proxy with metadata caching and rate limiting
//! 
//! Architecture:
//! - Downstream serves cached metadata via dav-server
//! - Upstream client fetches metadata and content with rate limiting
//! - Two single-threaded tasks: metadata_update and content_fetch

pub mod config;
pub mod webdav;
pub mod cache;
pub mod tasks;
pub mod resume;
pub mod webui;