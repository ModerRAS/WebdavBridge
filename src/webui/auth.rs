//! JWT authentication for WebUI
//!
//! - Login endpoint issues access + refresh tokens
//! - Refresh endpoint rotates tokens
//! - Auth middleware protects API endpoints

use axum::{
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Json, Response},
};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use super::state::AppState;

/// JWT claims
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: u64,
    pub token_type: String,
}

/// Login request body
#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// Token response
#[derive(Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub expires_in: u64,
}

/// Refresh request body
#[derive(Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

/// Error response
#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

const ACCESS_TOKEN_EXPIRY: u64 = 15 * 60; // 15 minutes
const REFRESH_TOKEN_EXPIRY: u64 = 7 * 24 * 60 * 60; // 7 days

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn create_token(secret: &str, sub: &str, token_type: &str, expiry_secs: u64) -> Result<String, jsonwebtoken::errors::Error> {
    let claims = Claims {
        sub: sub.to_string(),
        exp: now_secs() + expiry_secs,
        token_type: token_type.to_string(),
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
}

fn validate_token(secret: &str, token: &str, expected_type: &str) -> Result<Claims, StatusCode> {
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| StatusCode::UNAUTHORIZED)?;

    if token_data.claims.token_type != expected_type {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(token_data.claims)
}

/// POST /api/auth/login
pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> impl IntoResponse {
    let auth = &state.auth_state;

    if req.username != auth.username {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "Invalid credentials".to_string(),
            }),
        )
            .into_response();
    }

    match bcrypt::verify(&req.password, &auth.password_hash) {
        Ok(true) => {}
        _ => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    error: "Invalid credentials".to_string(),
                }),
            )
                .into_response();
        }
    }

    let access_token = match create_token(&auth.jwt_secret, &req.username, "access", ACCESS_TOKEN_EXPIRY) {
        Ok(t) => t,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Token creation failed".to_string(),
                }),
            )
                .into_response();
        }
    };

    let refresh_token = match create_token(&auth.refresh_secret, &req.username, "refresh", REFRESH_TOKEN_EXPIRY) {
        Ok(t) => t,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Token creation failed".to_string(),
                }),
            )
                .into_response();
        }
    };

    Json(TokenResponse {
        access_token,
        refresh_token,
        token_type: "Bearer".to_string(),
        expires_in: ACCESS_TOKEN_EXPIRY,
    })
    .into_response()
}

/// POST /api/auth/refresh
pub async fn refresh(
    State(state): State<AppState>,
    Json(req): Json<RefreshRequest>,
) -> impl IntoResponse {
    let auth = &state.auth_state;

    let claims = match validate_token(&auth.refresh_secret, &req.refresh_token, "refresh") {
        Ok(c) => c,
        Err(_) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    error: "Invalid refresh token".to_string(),
                }),
            )
                .into_response();
        }
    };

    let access_token = match create_token(&auth.jwt_secret, &claims.sub, "access", ACCESS_TOKEN_EXPIRY) {
        Ok(t) => t,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Token creation failed".to_string(),
                }),
            )
                .into_response();
        }
    };

    let refresh_token = match create_token(&auth.refresh_secret, &claims.sub, "refresh", REFRESH_TOKEN_EXPIRY) {
        Ok(t) => t,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Token creation failed".to_string(),
                }),
            )
                .into_response();
        }
    };

    Json(TokenResponse {
        access_token,
        refresh_token,
        token_type: "Bearer".to_string(),
        expires_in: ACCESS_TOKEN_EXPIRY,
    })
    .into_response()
}

/// POST /api/auth/logout
pub async fn logout() -> impl IntoResponse {
    // Stateless JWT - client should discard tokens
    StatusCode::OK
}

/// Authentication middleware
pub async fn auth_middleware(
    State(state): State<AppState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok());

    let token = match auth_header {
        Some(h) if h.starts_with("Bearer ") => &h[7..],
        _ => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    error: "Missing or invalid Authorization header".to_string(),
                }),
            )
                .into_response();
        }
    };

    match validate_token(&state.auth_state.jwt_secret, token, "access") {
        Ok(_) => next.run(req).await,
        Err(_) => (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "Invalid or expired token".to_string(),
            }),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_validate_token() {
        let secret = "test-secret-key";
        let token = create_token(secret, "admin", "access", 3600).unwrap();
        let claims = validate_token(secret, &token, "access").unwrap();
        assert_eq!(claims.sub, "admin");
        assert_eq!(claims.token_type, "access");
    }

    #[test]
    fn test_wrong_token_type_rejected() {
        let secret = "test-secret-key";
        let token = create_token(secret, "admin", "refresh", 3600).unwrap();
        let result = validate_token(secret, &token, "access");
        assert!(result.is_err());
    }

    #[test]
    fn test_wrong_secret_rejected() {
        let token = create_token("secret1", "admin", "access", 3600).unwrap();
        let result = validate_token("secret2", &token, "access");
        assert!(result.is_err());
    }

    #[test]
    fn test_expired_token_rejected() {
        let secret = "test-secret-key";
        // Create a token that's already expired (well past leeway)
        let claims = Claims {
            sub: "admin".to_string(),
            exp: now_secs() - 120,
            token_type: "access".to_string(),
        };
        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();
        let result = validate_token(secret, &token, "access");
        assert!(result.is_err());
    }
}
