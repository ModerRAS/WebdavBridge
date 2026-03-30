//! Symlinks API endpoints

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::webdav::types::WebdavResource;
use crate::webui::state::AppState;

/// Symlink entry
#[derive(Serialize)]
pub struct SymlinkEntry {
    pub path: String,
    pub name: String,
    pub target: Option<String>,
    pub has_local_override: bool,
    pub size: u64,
}

/// Create symlink request
#[derive(Deserialize)]
pub struct CreateSymlinkRequest {
    pub path: String,
    pub target: String,
}

/// Update symlink target request
#[derive(Deserialize)]
pub struct UpdateTargetRequest {
    pub target: String,
}

fn sanitize_path(path: &str) -> Result<String, StatusCode> {
    // Decode percent-encoded characters before checking for traversal
    let decoded = urlencoding_decode(path);
    let normalized = decoded.replace('\\', "/");
    if normalized.contains("..") {
        return Err(StatusCode::BAD_REQUEST);
    }
    let clean = if normalized.starts_with('/') {
        normalized
    } else {
        format!("/{}", normalized)
    };
    Ok(clean)
}

fn urlencoding_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let hi = chars.next();
            let lo = chars.next();
            if let (Some(h), Some(l)) = (hi, lo) {
                if let (Some(hv), Some(lv)) = (hex_val(h), hex_val(l)) {
                    result.push((hv << 4 | lv) as char);
                    continue;
                }
            }
            result.push('%');
        } else {
            result.push(b as char);
        }
    }
    result
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// GET /api/symlinks - List all symlinks
pub async fn list_symlinks(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let all: Vec<_> = state.metadata_cache.iter_all().await.collect();
    let symlinks: Vec<SymlinkEntry> = all
        .into_iter()
        .filter(|r| r.is_symlink)
        .map(|r| SymlinkEntry {
            path: r.path.clone(),
            name: r.name.clone(),
            target: r.symlink_target.clone(),
            has_local_override: r.has_local_override,
            size: r.size,
        })
        .collect();
    Json(symlinks)
}

/// POST /api/symlinks - Create a symlink
pub async fn create_symlink(
    State(state): State<AppState>,
    Json(req): Json<CreateSymlinkRequest>,
) -> impl IntoResponse {
    let path = match sanitize_path(&req.path) {
        Ok(p) => p,
        Err(s) => return (s, Json(serde_json::json!({"error": "Invalid path"}))).into_response(),
    };
    let target = match sanitize_path(&req.target) {
        Ok(t) => t,
        Err(s) => return (s, Json(serde_json::json!({"error": "Invalid target"}))).into_response(),
    };

    // Check for cycles
    let config = state.config.read().await;
    let max_depth = config.max_symlink_depth;
    drop(config);

    if let Err(e) = state.metadata_cache.check_symlink_safety(&path, &target, max_depth).await {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response();
    }

    // Get target resource info
    let target_resource = state.metadata_cache.get(&target).await;
    let (is_dir, size) = match &target_resource {
        Some(r) => (r.is_dir, r.size),
        None => (false, 0),
    };

    let name = path.rsplit('/').next().unwrap_or(&path).to_string();
    let symlink = WebdavResource::new_symlink(path.clone(), name, target.clone(), is_dir, size);

    if let Err(e) = state.metadata_cache.put(&symlink).await {
        tracing::error!("Failed to create symlink: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "Failed to create symlink"}))).into_response();
    }

    (StatusCode::CREATED, Json(serde_json::json!({"message": "Symlink created", "path": path}))).into_response()
}

/// DELETE /api/symlinks/*path - Delete a symlink
pub async fn delete_symlink(
    State(state): State<AppState>,
    Path(path): Path<String>,
) -> impl IntoResponse {
    let path = match sanitize_path(&path) {
        Ok(p) => p,
        Err(s) => return (s, Json(serde_json::json!({"error": "Invalid path"}))).into_response(),
    };

    // Verify it's a symlink
    match state.metadata_cache.get(&path).await {
        Some(r) if r.is_symlink => {}
        Some(_) => {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "Path is not a symlink"}))).into_response();
        }
        None => {
            return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Not found"}))).into_response();
        }
    }

    if let Err(e) = state.metadata_cache.delete(&path).await {
        tracing::error!("Failed to delete symlink: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "Failed to delete symlink"}))).into_response();
    }

    Json(serde_json::json!({"message": "Symlink deleted"})).into_response()
}

/// GET /api/symlinks/*path/target - Get symlink target
pub async fn get_symlink_target(
    State(state): State<AppState>,
    Path(path): Path<String>,
) -> impl IntoResponse {
    let path = match sanitize_path(&path) {
        Ok(p) => p,
        Err(s) => return (s, Json(serde_json::json!({"error": "Invalid path"}))).into_response(),
    };

    match state.metadata_cache.get_symlink_target(&path).await {
        Some(target) => Json(serde_json::json!({"path": path, "target": target})).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Not found or not a symlink"}))).into_response(),
    }
}

/// PUT /api/symlinks/*path/target - Update symlink target
pub async fn update_symlink_target(
    State(state): State<AppState>,
    Path(path): Path<String>,
    Json(req): Json<UpdateTargetRequest>,
) -> impl IntoResponse {
    let path = match sanitize_path(&path) {
        Ok(p) => p,
        Err(s) => return (s, Json(serde_json::json!({"error": "Invalid path"}))).into_response(),
    };
    let new_target = match sanitize_path(&req.target) {
        Ok(t) => t,
        Err(s) => return (s, Json(serde_json::json!({"error": "Invalid target"}))).into_response(),
    };

    // Get existing symlink
    let resource = match state.metadata_cache.get(&path).await {
        Some(r) if r.is_symlink => r,
        Some(_) => {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "Path is not a symlink"}))).into_response();
        }
        None => {
            return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Not found"}))).into_response();
        }
    };

    // Check for cycles
    let config = state.config.read().await;
    let max_depth = config.max_symlink_depth;
    drop(config);

    if let Err(e) = state.metadata_cache.check_symlink_safety(&path, &new_target, max_depth).await {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response();
    }

    // Update the resource
    let updated = WebdavResource::new_symlink(
        path.clone(),
        resource.name.clone(),
        new_target.clone(),
        resource.is_dir,
        resource.size,
    );

    if let Err(e) = state.metadata_cache.put(&updated).await {
        tracing::error!("Failed to update symlink: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "Failed to update symlink"}))).into_response();
    }

    Json(serde_json::json!({"message": "Symlink target updated", "path": path, "target": new_target})).into_response()
}
