//! WebdavBridge binary entry point
//! 
//! Wires together all components:
//! - Config loading
//! - Metadata cache (sled)
//! - Content cache (filesystem)
//! - Rate limiter (2 permits)
//! - Upstream client
//! - Metadata update task (single-threaded)
//! - Content fetch task (single-threaded)  
//! - WebDAV server (using hyper)

use std::net::SocketAddr;
use std::str;

use bytes::Bytes;
use http::header::HeaderValue;
use http::{Request, Response};
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::signal;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use webdav_bridge::webdav::server::WebdavServer;

#[derive(Clone)]
struct WebdavService {
    server: WebdavServer,
}

impl WebdavService {
    async fn handle_request(self, req: Request<Incoming>) -> Result<Response<Full<Bytes>>, hyper::Error> {
        let path = req.uri().path().to_string();
        let method = req.method().clone();
        
        let range_header = req.headers()
            .get("Range")
            .and_then(|v: &HeaderValue| v.to_str().ok())
            .map(|s: &str| s.to_string());
        
        let if_range_header = req.headers()
            .get("If-Range")
            .and_then(|v: &HeaderValue| v.to_str().ok());

        let destination_header = req.headers()
            .get("Destination")
            .and_then(|v: &HeaderValue| v.to_str().ok())
            .map(|s: &str| s.to_string());

        let overwrite_header = req.headers()
            .get("Overwrite")
            .and_then(|v: &HeaderValue| v.to_str().ok())
            .map(|s| s != "F")
            .unwrap_or(true);
        
        let response = match method {
            ref m if m.as_str() == "GET" => {
                match self.server.handle_get(&path, range_header.as_deref(), if_range_header).await {
                    Ok(get_resp) => {
                        let mut resp = Response::builder()
                            .status(get_resp.status)
                            .header("Content-Type", get_resp.content_type.unwrap_or_else(|| "application/octet-stream".to_string()));
                        if let Some(etag) = get_resp.etag {
                            resp = resp.header("ETag", etag);
                        }
                        if let Some(range) = get_resp.content_range {
                            resp = resp.header("Content-Range", range);
                            resp = resp.header("Content-Length", get_resp.bytes.len().to_string());
                        }
                        resp.body(Full::new(get_resp.bytes)).unwrap_or_else(|_| {
                            Response::builder().status(500).body(Full::new(Bytes::from("Internal error"))).unwrap()
                        })
                    }
                    Err(e) => {
                        tracing::warn!("GET {} failed: {}", path, e);
                        Response::builder().status(404).body(Full::new(Bytes::from("Not found"))).unwrap()
                    }
                }
            }
            ref m if m.as_str() == "HEAD" => {
                match self.server.handle_head(&path).await {
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
                        resp.body(Full::new(Bytes::new())).unwrap_or_else(|_| {
                            Response::builder().status(500).body(Full::new(Bytes::from("Internal error"))).unwrap()
                        })
                    }
                    Err(e) => {
                        tracing::warn!("HEAD {} failed: {}", path, e);
                        Response::builder().status(404).body(Full::new(Bytes::from("Not found"))).unwrap()
                    }
                }
            }
            ref m if m.as_str() == "PROPFIND" => {
                match self.server.handle_propfind(&path).await {
                    Ok(resources) => {
                        let xml = Self::build_propfind_response(&resources);
                        Response::builder()
                            .status(207)
                            .header("Content-Type", "application/xml")
                            .header("Depth", "1")
                            .body(Full::new(Bytes::from(xml)))
                            .unwrap_or_else(|_| {
                                Response::builder().status(500).body(Full::new(Bytes::from("Internal error"))).unwrap()
                            })
                    }
                    Err(e) => {
                        tracing::warn!("PROPFIND {} failed: {}", path, e);
                        Response::builder().status(404).body(Full::new(Bytes::from("Not found"))).unwrap()
                    }
                }
            }
            ref m if m.as_str() == "COPY" => {
                let dest = match Self::extract_dest_path(&destination_header) {
                    Some(d) => d,
                    None => {
                        return Ok(Response::builder().status(400).body(Full::new(Bytes::from("Missing or invalid Destination header"))).unwrap());
                    }
                };
                match self.server.handle_copy(&path, &dest, overwrite_header).await {
                    Ok(status) => {
                        Response::builder().status(status).body(Full::new(Bytes::new())).unwrap()
                    }
                    Err(webdav_bridge::webdav::types::WebdavError::NotFound(_)) => {
                        Response::builder().status(404).body(Full::new(Bytes::from("Not found"))).unwrap()
                    }
                    Err(webdav_bridge::webdav::types::WebdavError::PreconditionFailed(_)) => {
                        Response::builder().status(412).body(Full::new(Bytes::from("Precondition Failed"))).unwrap()
                    }
                    Err(webdav_bridge::webdav::types::WebdavError::SymlinkCycle(msg)) => {
                        Response::builder().status(400).body(Full::new(Bytes::from(msg))).unwrap()
                    }
                    Err(e) => {
                        tracing::warn!("COPY {} failed: {}", path, e);
                        Response::builder().status(500).body(Full::new(Bytes::from("Internal error"))).unwrap()
                    }
                }
            }
            ref m if m.as_str() == "MOVE" => {
                let dest = match Self::extract_dest_path(&destination_header) {
                    Some(d) => d,
                    None => {
                        return Ok(Response::builder().status(400).body(Full::new(Bytes::from("Missing or invalid Destination header"))).unwrap());
                    }
                };
                match self.server.handle_move(&path, &dest, overwrite_header).await {
                    Ok(status) => {
                        Response::builder().status(status).body(Full::new(Bytes::new())).unwrap()
                    }
                    Err(webdav_bridge::webdav::types::WebdavError::NotFound(_)) => {
                        Response::builder().status(404).body(Full::new(Bytes::from("Not found"))).unwrap()
                    }
                    Err(webdav_bridge::webdav::types::WebdavError::PreconditionFailed(_)) => {
                        Response::builder().status(412).body(Full::new(Bytes::from("Precondition Failed"))).unwrap()
                    }
                    Err(webdav_bridge::webdav::types::WebdavError::Forbidden(_)) => {
                        Response::builder().status(403).body(Full::new(Bytes::from("Forbidden"))).unwrap()
                    }
                    Err(e) => {
                        tracing::warn!("MOVE {} failed: {}", path, e);
                        Response::builder().status(500).body(Full::new(Bytes::from("Internal error"))).unwrap()
                    }
                }
            }
            ref m if m.as_str() == "PUT" => {
                use http_body_util::BodyExt;
                let body_bytes = match req.into_body().collect().await {
                    Ok(collected) => collected.to_bytes(),
                    Err(e) => {
                        tracing::warn!("PUT {} failed to read body: {}", path, e);
                        return Ok(Response::builder().status(400).body(Full::new(Bytes::from("Bad request"))).unwrap());
                    }
                };
                match self.server.handle_put(&path, body_bytes).await {
                    Ok(status) => {
                        Response::builder().status(status).body(Full::new(Bytes::new())).unwrap()
                    }
                    Err(webdav_bridge::webdav::types::WebdavError::NotFound(_)) => {
                        Response::builder().status(404).body(Full::new(Bytes::from("Not found"))).unwrap()
                    }
                    Err(webdav_bridge::webdav::types::WebdavError::Forbidden(_)) => {
                        Response::builder().status(403).body(Full::new(Bytes::from("Forbidden"))).unwrap()
                    }
                    Err(e) => {
                        tracing::warn!("PUT {} failed: {}", path, e);
                        Response::builder().status(500).body(Full::new(Bytes::from("Internal error"))).unwrap()
                    }
                }
            }
            ref m if m.as_str() == "DELETE" => {
                match self.server.handle_delete(&path).await {
                    Ok(status) => {
                        Response::builder().status(status).body(Full::new(Bytes::new())).unwrap()
                    }
                    Err(webdav_bridge::webdav::types::WebdavError::NotFound(_)) => {
                        Response::builder().status(404).body(Full::new(Bytes::from("Not found"))).unwrap()
                    }
                    Err(webdav_bridge::webdav::types::WebdavError::Forbidden(_)) => {
                        Response::builder().status(403).body(Full::new(Bytes::from("Forbidden"))).unwrap()
                    }
                    Err(e) => {
                        tracing::warn!("DELETE {} failed: {}", path, e);
                        Response::builder().status(500).body(Full::new(Bytes::from("Internal error"))).unwrap()
                    }
                }
            }
            _ => {
                Response::builder().status(405).body(Full::new(Bytes::from("Method not allowed"))).unwrap()
            }
        };
        
        Ok(response)
    }

    /// Extract destination path from the Destination header URL
    fn extract_dest_path(destination: &Option<String>) -> Option<String> {
        let dest = destination.as_deref()?;
        // Destination can be a full URL or just a path
        if let Ok(url) = url::Url::parse(dest) {
            Some(url.path().to_string())
        } else {
            // Treat as path directly
            Some(dest.to_string())
        }
    }
    
    fn build_propfind_response(resources: &[webdav_bridge::webdav::types::WebdavResource]) -> String {
        let mut xml = String::from(r#"<?xml version="1.0" encoding="utf-8" ?><D:multistatus xmlns:D="DAV:">"#);
        for resource in resources {
            let etag = resource.etag.as_ref().map(|e| format!(r#"<D:getetag>{}</D:getetag>"#, e)).unwrap_or_default();
            let last_modified = resource.modified.map(|dt| format!(r#"<D:getlastmodified>{}</D:getlastmodified>"#, dt.to_rfc2822())).unwrap_or_default();
            let content_type = resource.content_type.as_ref().map(|ct| format!(r#"<D:getcontenttype>{}</D:getcontenttype>"#, ct)).unwrap_or_default();
            
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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "webdav_bridge=debug,tokio=info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();
    
    tracing::info!("Starting WebdavBridge...");
    
    let config_path = std::env::var("WEBDAV_BRIDGE_CONFIG")
        .unwrap_or_else(|_| "config.yaml".to_string());
    
    let config = if std::path::Path::new(&config_path).exists() {
        webdav_bridge::config::load_config(&config_path)?
    } else {
        tracing::warn!("Config file '{}' not found, using defaults", config_path);
        webdav_bridge::config::Config::default()
    };
    
    tracing::info!("Config loaded: upstream={}", config.upstream_url);
    
    let rate_limiter = webdav_bridge::resume::RateLimiter::new(config.rate_limit_permits);
    
    let metadata_cache = webdav_bridge::cache::metadata::MetadataCache::open(&config.metadata_db_path).await?;
    tracing::info!("Metadata cache opened: {:?}", config.metadata_db_path);
    
    let mut cache_dir = config.cache_dir.clone();
    if !cache_dir.is_absolute() {
        cache_dir = std::env::current_dir()?.join(&cache_dir);
    }
    let content_cache = webdav_bridge::cache::content::ContentCache::new(&cache_dir);
    content_cache.ensure_dir().await?;
    tracing::info!("Content cache initialized: {:?}", cache_dir);
    
    let upstream = webdav_bridge::webdav::client::UpstreamClient::new(&config, rate_limiter.clone())?;
    tracing::info!("Upstream client initialized");
    
    let content_fetch = webdav_bridge::tasks::content_fetch::ContentFetchTask::new(
        upstream.clone(),
        content_cache,
        rate_limiter.clone(),
    );

    let content_fetch_for_server = content_fetch.clone();
    let (content_fetch_handle, _work_tx, content_fetch_stop_tx) = content_fetch.start().await;
    tracing::info!("Content fetch task started");

    let webdav_server = webdav_bridge::webdav::server::WebdavServer::new(
        content_fetch_for_server,
        metadata_cache.clone(),
    )
    .with_content_cache(webdav_bridge::cache::content::ContentCache::new(&cache_dir))
    .with_max_symlink_depth(config.max_symlink_depth);

    let metadata_update = webdav_bridge::tasks::metadata_update::MetadataUpdateTask::new(
        upstream,
        metadata_cache,
        rate_limiter,
        config.metadata_update_interval_secs,
        config.max_depth,
    );
    let (update_handle, stop_tx) = metadata_update.start();
    tracing::info!("Metadata update task started");
    
    let bind_addr: SocketAddr = config.server_bind.parse()?;
    let listener = TcpListener::bind(bind_addr).await?;
    tracing::info!("Server listening on {}", bind_addr);
    
    let shutdown = async {
        signal::ctrl_c().await.expect("Failed to listen for Ctrl+C");
        tracing::info!("Received Ctrl+C, shutting down...");
    };
    
    tokio::pin!(shutdown);
    
    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, remote_addr) = match result {
                    Ok((stream, addr)) => (stream, addr),
                    Err(e) => {
                        tracing::warn!("Failed to accept connection: {}", e);
                        continue;
                    }
                };
                
                let service = WebdavService {
                    server: webdav_server.clone(),
                };
                
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    
                    if let Err(err) = http1::Builder::new()
                        .serve_connection(
                            io,
                            service_fn(move |req| {
                                let service = service.clone();
                                async move { service.handle_request(req).await }
                            }),
                        )
                        .await
                    {
                        tracing::warn!("Failed to serve connection {}: {}", remote_addr, err);
                    }
                });
            }
            _ = &mut shutdown => {
                let _ = content_fetch_stop_tx.send(true);
                let _ = stop_tx.send(true);
                tracing::info!("Stopping tasks...");
                break;
            }
        }
    }

    tracing::info!("Waiting for tasks to finish...");
    let _ = content_fetch_handle.await;
    let _ = update_handle.await;
    tracing::info!("WebdavBridge stopped");
    Ok(())
}
