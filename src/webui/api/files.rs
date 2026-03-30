//! Files API endpoints - WebDAV operations via REST

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::webdav::types::WebdavError;
use crate::webui::state::{AppState, StatusEvent};

/// File entry in directory listing
#[derive(Serialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub content_type: Option<String>,
    pub etag: Option<String>,
    pub modified: Option<String>,
    pub is_symlink: bool,
    pub symlink_target: Option<String>,
    pub has_local_override: bool,
}

#[derive(Deserialize)]
pub struct MoveRequest {
    pub destination: String,
}

#[derive(Deserialize)]
pub struct CopyRequest {
    pub destination: String,
}

#[derive(Deserialize, Default)]
pub struct ListQuery {
    pub page: Option<usize>,
    pub per_page: Option<usize>,
}

fn sanitize_path(path: &str) -> Result<String, StatusCode> {
    // Decode percent-encoded characters before checking for traversal
    let decoded = urlencoding_decode(path);
    let normalized = decoded.replace('\\', "/");
    // Prevent path traversal (including percent-encoded variants)
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

/// GET /api/files/*path - List directory or get file info
pub async fn list_files(
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(query): Query<ListQuery>,
) -> impl IntoResponse {
    let path = match sanitize_path(&path) {
        Ok(p) => p,
        Err(s) => return (s, Json(serde_json::json!({"error": "Invalid path"}))).into_response(),
    };

    let children = state.metadata_cache.get_children(&path).await;

    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(100).min(1000);
    let start = (page - 1) * per_page;

    let total = children.len();
    let entries: Vec<FileEntry> = children
        .into_iter()
        .skip(start)
        .take(per_page)
        .map(|r| FileEntry {
            name: r.name.clone(),
            path: r.path.clone(),
            is_dir: r.is_dir,
            size: r.size,
            content_type: r.content_type.clone(),
            etag: r.etag.clone(),
            modified: r.modified.map(|d| d.to_rfc3339()),
            is_symlink: r.is_symlink,
            symlink_target: r.symlink_target.clone(),
            has_local_override: r.has_local_override,
        })
        .collect();

    Json(serde_json::json!({
        "path": path,
        "entries": entries,
        "total": total,
        "page": page,
        "per_page": per_page,
    }))
    .into_response()
}

/// GET /api/files/*path/download - Download file content
pub async fn download_file(
    State(state): State<AppState>,
    Path(path): Path<String>,
) -> impl IntoResponse {
    let path = match sanitize_path(&path) {
        Ok(p) => p,
        Err(s) => return (s, "Invalid path").into_response(),
    };

    match state.webdav_server.handle_get(&path, None, None).await {
        Ok(resp) => {
            let content_type = resp
                .content_type
                .unwrap_or_else(|| "application/octet-stream".to_string());
            let mut builder = axum::http::Response::builder()
                .status(200)
                .header("Content-Type", content_type)
                .header("Content-Length", resp.bytes.len().to_string());
            if let Some(etag) = resp.etag {
                builder = builder.header("ETag", etag);
            }
            builder
                .body(axum::body::Body::from(resp.bytes))
                .unwrap()
                .into_response()
        }
        Err(WebdavError::NotFound(_)) => {
            (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Not found"}))).into_response()
        }
        Err(e) => {
            tracing::warn!("Download failed for {}: {}", path, e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "Internal error"}))).into_response()
        }
    }
}

/// PUT /api/files/*path - Upload file
pub async fn upload_file(
    State(state): State<AppState>,
    Path(path): Path<String>,
    body: Bytes,
) -> impl IntoResponse {
    let path = match sanitize_path(&path) {
        Ok(p) => p,
        Err(s) => return (s, Json(serde_json::json!({"error": "Invalid path"}))).into_response(),
    };

    match state.webdav_server.handle_put(&path, body).await {
        Ok(status) => {
            let _ = state.status_tx.send(StatusEvent::CacheUpdate {
                path: path.clone(),
                action: "upload".to_string(),
            });
            (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), Json(serde_json::json!({"message": "File uploaded"}))).into_response()
        }
        Err(WebdavError::NotFound(_)) => {
            (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Not found"}))).into_response()
        }
        Err(WebdavError::Forbidden(msg)) => {
            (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": msg}))).into_response()
        }
        Err(e) => {
            tracing::warn!("Upload failed for {}: {}", path, e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "Internal error"}))).into_response()
        }
    }
}

/// DELETE /api/files/*path - Delete file or directory
pub async fn delete_file(
    State(state): State<AppState>,
    Path(path): Path<String>,
) -> impl IntoResponse {
    let path = match sanitize_path(&path) {
        Ok(p) => p,
        Err(s) => return (s, Json(serde_json::json!({"error": "Invalid path"}))).into_response(),
    };

    match state.webdav_server.handle_delete(&path).await {
        Ok(status) => {
            let _ = state.status_tx.send(StatusEvent::CacheUpdate {
                path: path.clone(),
                action: "delete".to_string(),
            });
            (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), Json(serde_json::json!({"message": "Deleted"}))).into_response()
        }
        Err(WebdavError::NotFound(_)) => {
            (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Not found"}))).into_response()
        }
        Err(WebdavError::Forbidden(msg)) => {
            (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": msg}))).into_response()
        }
        Err(e) => {
            tracing::warn!("Delete failed for {}: {}", path, e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "Internal error"}))).into_response()
        }
    }
}

/// POST /api/files/*path/copy - Copy file
pub async fn copy_file(
    State(state): State<AppState>,
    Path(path): Path<String>,
    Json(req): Json<CopyRequest>,
) -> impl IntoResponse {
    let path = match sanitize_path(&path) {
        Ok(p) => p,
        Err(s) => return (s, Json(serde_json::json!({"error": "Invalid path"}))).into_response(),
    };
    let dest = match sanitize_path(&req.destination) {
        Ok(p) => p,
        Err(s) => return (s, Json(serde_json::json!({"error": "Invalid destination"}))).into_response(),
    };

    match state.webdav_server.handle_copy(&path, &dest, true).await {
        Ok(status) => {
            let _ = state.status_tx.send(StatusEvent::CacheUpdate {
                path: dest.clone(),
                action: "copy".to_string(),
            });
            (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), Json(serde_json::json!({"message": "Copied"}))).into_response()
        }
        Err(WebdavError::NotFound(_)) => {
            (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Not found"}))).into_response()
        }
        Err(WebdavError::SymlinkCycle(msg)) => {
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": msg}))).into_response()
        }
        Err(e) => {
            tracing::warn!("Copy failed for {}: {}", path, e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "Internal error"}))).into_response()
        }
    }
}

/// POST /api/files/*path/move - Move/rename file
pub async fn move_file(
    State(state): State<AppState>,
    Path(path): Path<String>,
    Json(req): Json<MoveRequest>,
) -> impl IntoResponse {
    let path = match sanitize_path(&path) {
        Ok(p) => p,
        Err(s) => return (s, Json(serde_json::json!({"error": "Invalid path"}))).into_response(),
    };
    let dest = match sanitize_path(&req.destination) {
        Ok(p) => p,
        Err(s) => return (s, Json(serde_json::json!({"error": "Invalid destination"}))).into_response(),
    };

    match state.webdav_server.handle_move(&path, &dest, true).await {
        Ok(status) => {
            let _ = state.status_tx.send(StatusEvent::CacheUpdate {
                path: dest.clone(),
                action: "move".to_string(),
            });
            (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), Json(serde_json::json!({"message": "Moved"}))).into_response()
        }
        Err(WebdavError::NotFound(_)) => {
            (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Not found"}))).into_response()
        }
        Err(WebdavError::Forbidden(msg)) => {
            (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": msg}))).into_response()
        }
        Err(e) => {
            tracing::warn!("Move failed for {}: {}", path, e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "Internal error"}))).into_response()
        }
    }
}
