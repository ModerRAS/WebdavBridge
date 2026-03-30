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
//! - Axum HTTP server with WebDAV + WebUI

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::signal;
use tokio::sync::{broadcast, RwLock};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use webdav_bridge::webui::state::{AppState, AuthState};
use webdav_bridge::webui::router::build_router;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "webdav_bridge=debug,tower_http=debug,tokio=info".into()))
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
        webdav_bridge::cache::content::ContentCache::new(&cache_dir),
        rate_limiter.clone(),
    );

    let content_fetch_for_server = content_fetch.clone();
    let content_fetch_for_state = content_fetch.clone();
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
        metadata_cache.clone(),
        rate_limiter,
        config.metadata_update_interval_secs,
        config.max_depth,
    );
    let (update_handle, stop_tx) = metadata_update.start();
    tracing::info!("Metadata update task started");
    
    // Setup auth state
    let webui_username = std::env::var("WEBUI_USERNAME").unwrap_or_else(|_| "admin".to_string());
    let webui_password = std::env::var("WEBUI_PASSWORD").unwrap_or_else(|_| "admin".to_string());
    let password_hash = bcrypt::hash(&webui_password, bcrypt::DEFAULT_COST)
        .expect("Failed to hash password");
    
    let jwt_secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| {
        uuid::Uuid::new_v4().to_string()
    });
    let refresh_secret = std::env::var("JWT_REFRESH_SECRET").unwrap_or_else(|_| {
        uuid::Uuid::new_v4().to_string()
    });

    let (status_tx, _) = broadcast::channel(100);

    let app_state = AppState {
        webdav_server,
        metadata_cache: Arc::new(metadata_cache),
        content_cache: Arc::new(content_cache),
        content_fetch: Arc::new(content_fetch_for_state),
        config: Arc::new(RwLock::new(config.clone())),
        config_path,
        auth_state: Arc::new(AuthState {
            jwt_secret,
            refresh_secret,
            password_hash,
            username: webui_username.clone(),
        }),
        status_tx,
    };

    let app = build_router(app_state);
    
    let bind_addr: SocketAddr = config.server_bind.parse()?;
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    tracing::info!("Server listening on {}", bind_addr);
    tracing::info!("WebUI: http://{}", bind_addr);
    tracing::info!("WebDAV: http://{}/webdav/", bind_addr);
    tracing::info!("Health: http://{}/health", bind_addr);
    tracing::info!("WebUI username: {}", webui_username);
    
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            signal::ctrl_c().await.expect("Failed to listen for Ctrl+C");
            tracing::info!("Received Ctrl+C, shutting down...");
        })
        .await?;

    let _ = content_fetch_stop_tx.send(true);
    let _ = stop_tx.send(true);
    tracing::info!("Stopping tasks...");

    tracing::info!("Waiting for tasks to finish...");
    let _ = content_fetch_handle.await;
    let _ = update_handle.await;
    tracing::info!("WebdavBridge stopped");
    Ok(())
}
