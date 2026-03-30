//! Configuration API endpoints

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::webui::state::AppState;

/// Serializable config representation (hides sensitive fields)
#[derive(Serialize, Deserialize)]
pub struct ConfigResponse {
    pub upstream_url: String,
    pub upstream_username: Option<String>,
    #[serde(skip_serializing)]
    pub upstream_password: Option<String>,
    pub cache_dir: String,
    pub metadata_db_path: String,
    pub rate_limit_permits: usize,
    pub metadata_update_interval_secs: u64,
    pub max_depth: u32,
    pub server_bind: String,
    pub server_prefix: String,
    pub max_symlink_depth: u32,
}

/// Partial config update request
#[derive(Deserialize)]
pub struct ConfigUpdateRequest {
    pub upstream_url: Option<String>,
    pub upstream_username: Option<String>,
    pub upstream_password: Option<String>,
    pub cache_dir: Option<String>,
    pub rate_limit_permits: Option<usize>,
    pub metadata_update_interval_secs: Option<u64>,
    pub max_depth: Option<u32>,
    pub server_prefix: Option<String>,
    pub max_symlink_depth: Option<u32>,
}

/// GET /api/config
pub async fn get_config(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let config = state.config.read().await;
    let resp = ConfigResponse {
        upstream_url: config.upstream_url.clone(),
        upstream_username: config.upstream_username.clone(),
        upstream_password: None, // Never expose password
        cache_dir: config.cache_dir.to_string_lossy().to_string(),
        metadata_db_path: config.metadata_db_path.to_string_lossy().to_string(),
        rate_limit_permits: config.rate_limit_permits,
        metadata_update_interval_secs: config.metadata_update_interval_secs,
        max_depth: config.max_depth,
        server_bind: config.server_bind.clone(),
        server_prefix: config.server_prefix.clone(),
        max_symlink_depth: config.max_symlink_depth,
    };
    Json(resp)
}

/// PUT /api/config
pub async fn update_config(
    State(state): State<AppState>,
    Json(update): Json<ConfigUpdateRequest>,
) -> impl IntoResponse {
    let mut config = state.config.write().await;

    if let Some(url) = update.upstream_url {
        if url.is_empty() {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "upstream_url cannot be empty"}))).into_response();
        }
        config.upstream_url = url;
    }
    if let Some(username) = update.upstream_username {
        config.upstream_username = Some(username);
    }
    if let Some(password) = update.upstream_password {
        config.upstream_password = Some(password);
    }
    if let Some(dir) = update.cache_dir {
        config.cache_dir = dir.into();
    }
    if let Some(permits) = update.rate_limit_permits {
        if permits == 0 {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "rate_limit_permits must be > 0"}))).into_response();
        }
        config.rate_limit_permits = permits;
    }
    if let Some(interval) = update.metadata_update_interval_secs {
        config.metadata_update_interval_secs = interval;
    }
    if let Some(depth) = update.max_depth {
        config.max_depth = depth;
    }
    if let Some(prefix) = update.server_prefix {
        config.server_prefix = prefix;
    }
    if let Some(depth) = update.max_symlink_depth {
        config.max_symlink_depth = depth;
    }

    // Persist to file
    if let Err(e) = crate::config::save_config(&config, &state.config_path) {
        tracing::error!("Failed to save config: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "Failed to save config"}))).into_response();
    }

    (StatusCode::OK, Json(serde_json::json!({"message": "Config updated"}))).into_response()
}
