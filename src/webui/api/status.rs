//! Status API endpoints

use axum::{
    extract::State,
    response::IntoResponse,
    Json,
};
use serde::Serialize;

use crate::webui::state::AppState;

/// System status response
#[derive(Serialize)]
pub struct StatusResponse {
    pub server: ServerStatus,
    pub cache: CacheStatus,
}

#[derive(Serialize)]
pub struct ServerStatus {
    pub uptime_secs: u64,
    pub version: String,
    pub upstream_url: String,
}

#[derive(Serialize)]
pub struct CacheStatus {
    pub metadata_entries: usize,
    pub symlink_count: usize,
}

/// GET /api/status
pub async fn get_status(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let config = state.config.read().await;
    
    // Count metadata entries
    let all_resources: Vec<_> = state.metadata_cache.iter_all().await.collect();
    let total = all_resources.len();
    let symlinks = all_resources.iter().filter(|r| r.is_symlink).count();

    let resp = StatusResponse {
        server: ServerStatus {
            uptime_secs: 0, // Could track start time
            version: env!("CARGO_PKG_VERSION").to_string(),
            upstream_url: config.upstream_url.clone(),
        },
        cache: CacheStatus {
            metadata_entries: total,
            symlink_count: symlinks,
        },
    };
    Json(resp)
}

/// GET /api/status/stats
pub async fn get_stats(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let all_resources: Vec<_> = state.metadata_cache.iter_all().await.collect();
    let total = all_resources.len();
    let dirs = all_resources.iter().filter(|r| r.is_dir).count();
    let files = all_resources.iter().filter(|r| !r.is_dir && !r.is_symlink).count();
    let symlinks = all_resources.iter().filter(|r| r.is_symlink).count();
    let overrides = all_resources.iter().filter(|r| r.has_local_override).count();
    let total_size: u64 = all_resources.iter().map(|r| r.size).sum();

    Json(serde_json::json!({
        "total_entries": total,
        "directories": dirs,
        "files": files,
        "symlinks": symlinks,
        "local_overrides": overrides,
        "total_size_bytes": total_size,
    }))
}
