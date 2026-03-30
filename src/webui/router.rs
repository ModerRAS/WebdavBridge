//! Axum router construction
//!
//! Builds the complete router with:
//! - WebDAV endpoints (preserving existing behavior)
//! - API endpoints (protected by JWT auth)
//! - WebSocket endpoint
//! - Static file serving (SPA fallback)
//! - Health check

use axum::{
    body::Body,
    extract::State,
    http::{header::HeaderValue, Method, Request, Response, StatusCode},
    middleware,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use http_body_util::BodyExt;
use rust_embed::Embed;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use super::api::{config as config_api, files, status, symlinks};
use super::auth;
use super::state::AppState;
use super::ws;

/// Embedded frontend assets
#[derive(Embed)]
#[folder = "frontend/dist/"]
struct FrontendAssets;

/// Build the complete application router
pub fn build_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_origin(Any)
        .allow_headers(Any);

    // Auth routes (public)
    let auth_routes = Router::new()
        .route("/login", post(auth::login))
        .route("/logout", post(auth::logout))
        .route("/refresh", post(auth::refresh));

    // Protected API routes
    let api_routes = Router::new()
        .route("/config", get(config_api::get_config).put(config_api::update_config))
        .route("/status", get(status::get_status))
        .route("/status/stats", get(status::get_stats))
        .route("/symlinks", get(symlinks::list_symlinks).post(symlinks::create_symlink))
        .route("/symlinks/*path", delete(symlinks::delete_symlink))
        .route("/symlinks/*path/target", get(symlinks::get_symlink_target).put(symlinks::update_symlink_target))
        .route("/files/*path", get(files::list_files).put(files::upload_file).delete(files::delete_file))
        .route("/files/*path/download", get(files::download_file))
        .route("/files/*path/copy", post(files::copy_file))
        .route("/files/*path/move", post(files::move_file))
        .layer(middleware::from_fn_with_state(state.clone(), auth::auth_middleware));

    // WebDAV routes (preserving original behavior)
    let webdav_routes = Router::new()
        .fallback(webdav_handler);

    Router::new()
        .route("/health", get(health_check))
        .route("/ws", get(ws::ws_handler))
        .nest("/api/auth", auth_routes)
        .nest("/api", api_routes)
        .nest("/webdav", webdav_routes)
        .fallback(static_handler)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Health check endpoint
async fn health_check() -> impl IntoResponse {
    Json(serde_json::json!({"status": "ok"}))
}

/// Static file handler with SPA fallback
async fn static_handler(req: Request<Body>) -> impl IntoResponse {
    let path = req.uri().path().trim_start_matches('/');

    // Try to serve static file
    if let Some(content) = FrontendAssets::get(path) {
        let mime = mime_guess::from_path(path)
            .first_or_octet_stream()
            .to_string();
        return Response::builder()
            .status(200)
            .header("Content-Type", mime)
            .header("Cache-Control", "public, max-age=3600")
            .body(Body::from(content.data.to_vec()))
            .unwrap()
            .into_response();
    }

    // SPA fallback: serve index.html for non-API routes
    if let Some(content) = FrontendAssets::get("index.html") {
        return Response::builder()
            .status(200)
            .header("Content-Type", "text/html")
            .body(Body::from(content.data.to_vec()))
            .unwrap()
            .into_response();
    }

    // No frontend built yet
    (StatusCode::OK, "WebdavBridge is running. Build the frontend to see the WebUI.").into_response()
}

/// WebDAV passthrough handler - preserves all original WebDAV behavior
async fn webdav_handler(
    State(state): State<AppState>,
    req: Request<Body>,
) -> impl IntoResponse {
    let path = req.uri().path().to_string();
    // Strip /webdav prefix if present
    let webdav_path = path.strip_prefix("/webdav").unwrap_or(&path);
    let webdav_path = if webdav_path.is_empty() { "/" } else { webdav_path };

    let method = req.method().clone();
    let range_header = req
        .headers()
        .get("Range")
        .and_then(|v: &HeaderValue| v.to_str().ok())
        .map(|s: &str| s.to_string());
    let if_range_header = req
        .headers()
        .get("If-Range")
        .and_then(|v: &HeaderValue| v.to_str().ok())
        .map(|s| s.to_string());
    let destination_header = req
        .headers()
        .get("Destination")
        .and_then(|v: &HeaderValue| v.to_str().ok())
        .map(|s: &str| s.to_string());
    let overwrite_header = req
        .headers()
        .get("Overwrite")
        .and_then(|v: &HeaderValue| v.to_str().ok())
        .map(|s| s != "F")
        .unwrap_or(true);

    let server = &state.webdav_server;

    match method {
        Method::GET => {
            match server
                .handle_get(webdav_path, range_header.as_deref(), if_range_header.as_deref())
                .await
            {
                Ok(get_resp) => {
                    let mut resp = Response::builder()
                        .status(get_resp.status)
                        .header(
                            "Content-Type",
                            get_resp
                                .content_type
                                .unwrap_or_else(|| "application/octet-stream".to_string()),
                        );
                    if let Some(etag) = get_resp.etag {
                        resp = resp.header("ETag", etag);
                    }
                    if let Some(range) = get_resp.content_range {
                        resp = resp.header("Content-Range", range);
                        resp = resp.header("Content-Length", get_resp.bytes.len().to_string());
                    }
                    resp.body(Body::from(get_resp.bytes))
                        .unwrap_or_else(|_| {
                            Response::builder()
                                .status(500)
                                .body(Body::from("Internal error"))
                                .unwrap()
                        })
                        .into_response()
                }
                Err(e) => {
                    tracing::warn!("GET {} failed: {}", webdav_path, e);
                    (StatusCode::NOT_FOUND, "Not found").into_response()
                }
            }
        }
        ref m if m.as_str() == "HEAD" => {
            match server.handle_head(webdav_path).await {
                Ok(head_resp) => {
                    let mut resp = Response::builder()
                        .header("Content-Length", head_resp.size.to_string());
                    if head_resp.supports_range {
                        resp = resp.header("Accept-Ranges", "bytes");
                    }
                    if let Some(etag) = head_resp.etag {
                        resp = resp.header("ETag", etag);
                    }
                    if let Some(ct) = head_resp.content_type {
                        resp = resp.header("Content-Type", ct);
                    }
                    resp.body(Body::empty())
                        .unwrap_or_else(|_| {
                            Response::builder()
                                .status(500)
                                .body(Body::from("Internal error"))
                                .unwrap()
                        })
                        .into_response()
                }
                Err(e) => {
                    tracing::warn!("HEAD {} failed: {}", webdav_path, e);
                    (StatusCode::NOT_FOUND, "Not found").into_response()
                }
            }
        }
        ref m if m.as_str() == "PROPFIND" => {
            match server.handle_propfind(webdav_path).await {
                Ok(resources) => {
                    let xml = build_propfind_response(&resources);
                    Response::builder()
                        .status(207)
                        .header("Content-Type", "application/xml")
                        .header("Depth", "1")
                        .body(Body::from(xml))
                        .unwrap_or_else(|_| {
                            Response::builder()
                                .status(500)
                                .body(Body::from("Internal error"))
                                .unwrap()
                        })
                        .into_response()
                }
                Err(e) => {
                    tracing::warn!("PROPFIND {} failed: {}", webdav_path, e);
                    (StatusCode::NOT_FOUND, "Not found").into_response()
                }
            }
        }
        ref m if m.as_str() == "COPY" => {
            let dest = match extract_dest_path(&destination_header) {
                Some(d) => d,
                None => {
                    return (StatusCode::BAD_REQUEST, "Missing or invalid Destination header")
                        .into_response()
                }
            };
            match server.handle_copy(webdav_path, &dest, overwrite_header).await {
                Ok(status) => Response::builder()
                    .status(status)
                    .body(Body::empty())
                    .unwrap()
                    .into_response(),
                Err(crate::webdav::types::WebdavError::NotFound(_)) => {
                    (StatusCode::NOT_FOUND, "Not found").into_response()
                }
                Err(crate::webdav::types::WebdavError::PreconditionFailed(_)) => {
                    (StatusCode::PRECONDITION_FAILED, "Precondition Failed").into_response()
                }
                Err(crate::webdav::types::WebdavError::SymlinkCycle(msg)) => {
                    (StatusCode::BAD_REQUEST, msg).into_response()
                }
                Err(crate::webdav::types::WebdavError::SymlinkDepthExceeded { max_depth }) => {
                    (
                        StatusCode::BAD_REQUEST,
                        format!("Symlink depth exceeded: max depth is {}", max_depth),
                    )
                        .into_response()
                }
                Err(e) => {
                    tracing::warn!("COPY {} failed: {}", webdav_path, e);
                    (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
                }
            }
        }
        ref m if m.as_str() == "MOVE" => {
            let dest = match extract_dest_path(&destination_header) {
                Some(d) => d,
                None => {
                    return (StatusCode::BAD_REQUEST, "Missing or invalid Destination header")
                        .into_response()
                }
            };
            match server.handle_move(webdav_path, &dest, overwrite_header).await {
                Ok(status) => Response::builder()
                    .status(status)
                    .body(Body::empty())
                    .unwrap()
                    .into_response(),
                Err(crate::webdav::types::WebdavError::NotFound(_)) => {
                    (StatusCode::NOT_FOUND, "Not found").into_response()
                }
                Err(crate::webdav::types::WebdavError::PreconditionFailed(_)) => {
                    (StatusCode::PRECONDITION_FAILED, "Precondition Failed").into_response()
                }
                Err(crate::webdav::types::WebdavError::Forbidden(_)) => {
                    (StatusCode::FORBIDDEN, "Forbidden").into_response()
                }
                Err(e) => {
                    tracing::warn!("MOVE {} failed: {}", webdav_path, e);
                    (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
                }
            }
        }
        Method::PUT => {
            let body_bytes = match req.into_body().collect().await {
                Ok(collected) => collected.to_bytes(),
                Err(e) => {
                    tracing::warn!("PUT {} failed to read body: {}", webdav_path, e);
                    return (StatusCode::BAD_REQUEST, "Bad request").into_response();
                }
            };
            match server.handle_put(webdav_path, body_bytes).await {
                Ok(status) => Response::builder()
                    .status(status)
                    .body(Body::empty())
                    .unwrap()
                    .into_response(),
                Err(crate::webdav::types::WebdavError::NotFound(_)) => {
                    (StatusCode::NOT_FOUND, "Not found").into_response()
                }
                Err(crate::webdav::types::WebdavError::Forbidden(_)) => {
                    (StatusCode::FORBIDDEN, "Forbidden").into_response()
                }
                Err(e) => {
                    tracing::warn!("PUT {} failed: {}", webdav_path, e);
                    (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
                }
            }
        }
        Method::DELETE => match server.handle_delete(webdav_path).await {
            Ok(status) => Response::builder()
                .status(status)
                .body(Body::empty())
                .unwrap()
                .into_response(),
            Err(crate::webdav::types::WebdavError::NotFound(_)) => {
                (StatusCode::NOT_FOUND, "Not found").into_response()
            }
            Err(crate::webdav::types::WebdavError::Forbidden(_)) => {
                (StatusCode::FORBIDDEN, "Forbidden").into_response()
            }
            Err(e) => {
                tracing::warn!("DELETE {} failed: {}", webdav_path, e);
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
            }
        },
        _ => (StatusCode::METHOD_NOT_ALLOWED, "Method not allowed").into_response(),
    }
}

fn extract_dest_path(destination: &Option<String>) -> Option<String> {
    let dest = destination.as_deref()?;
    if let Ok(url) = url::Url::parse(dest) {
        Some(url.path().to_string())
    } else {
        Some(dest.to_string())
    }
}

fn build_propfind_response(
    resources: &[crate::webdav::types::WebdavResource],
) -> String {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="utf-8" ?><D:multistatus xmlns:D="DAV:">"#,
    );
    for resource in resources {
        let etag = resource
            .etag
            .as_ref()
            .map(|e| format!(r#"<D:getetag>{}</D:getetag>"#, e))
            .unwrap_or_default();
        let last_modified = resource
            .modified
            .map(|dt| {
                format!(
                    r#"<D:getlastmodified>{}</D:getlastmodified>"#,
                    dt.to_rfc2822()
                )
            })
            .unwrap_or_default();
        let content_type = resource
            .content_type
            .as_ref()
            .map(|ct| format!(r#"<D:getcontenttype>{}</D:getcontenttype>"#, ct))
            .unwrap_or_default();

        xml.push_str(&format!(
            r#"<D:response><D:href>{}</D:href><D:propstat><D:prop><D:displayname>{}</D:displayname><D:getcontentlength>{}</D:getcontentlength><D:resourcetype>{}</D:resourcetype>{}{}{}</D:prop></D:propstat></D:response>"#,
            resource.path,
            resource.name,
            resource.size,
            if resource.is_dir { "<D:collection/>" } else { "" },
            etag,
            last_modified,
            content_type
        ));
    }
    xml.push_str("</D:multistatus>");
    xml
}
