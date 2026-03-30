//! Shared application state for the WebUI

use crate::cache::content::ContentCache;
use crate::cache::metadata::MetadataCache;
use crate::config::Config;
use crate::tasks::content_fetch::ContentFetchTask;
use crate::webdav::server::WebdavServer;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

/// Shared application state accessible from all handlers
#[derive(Clone)]
pub struct AppState {
    pub webdav_server: WebdavServer,
    pub metadata_cache: Arc<MetadataCache>,
    pub content_cache: Arc<ContentCache>,
    pub content_fetch: Arc<ContentFetchTask>,
    pub config: Arc<RwLock<Config>>,
    pub config_path: String,
    pub auth_state: Arc<AuthState>,
    pub status_tx: broadcast::Sender<StatusEvent>,
}

/// Authentication state
pub struct AuthState {
    pub jwt_secret: String,
    pub refresh_secret: String,
    pub password_hash: String,
    pub username: String,
}

/// Status events pushed via WebSocket
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "type")]
pub enum StatusEvent {
    #[serde(rename = "connection_status")]
    ConnectionStatus { connected: bool },
    #[serde(rename = "cache_update")]
    CacheUpdate { path: String, action: String },
    #[serde(rename = "task_status")]
    TaskStatus { task: String, status: String },
}
