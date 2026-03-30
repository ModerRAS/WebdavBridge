//! WebUI module - provides REST API, WebSocket, and static file serving
//!
//! Submodules:
//! - `auth`: JWT authentication and middleware
//! - `api`: REST API endpoints (config, status, files, symlinks)
//! - `ws`: WebSocket infrastructure for real-time status
//! - `state`: Shared application state
//! - `router`: Axum router construction

pub mod auth;
pub mod api;
pub mod ws;
pub mod state;
pub mod router;
