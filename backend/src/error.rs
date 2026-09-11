//! A tiny error type that renders as JSON so the frontend always gets a
//! predictable `{ "error": "..." }` shape.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(Debug)]
pub enum ApiError {
    NotFound(String),
    BadRequest(String),
    Conflict(String),
    /// Refused by a safety policy — an unsafe disk, or a destructive operation
    /// attempted while in dry-run mode.
    Forbidden(String),
    /// Not authenticated (no session, or an expired one).
    Unauthorized(String),
    /// Too many failed login attempts.
    TooManyRequests(String),
}

impl ApiError {
    /// The human-readable message, regardless of status — the same text the
    /// JSON body carries. Useful for logging an `ApiError` without matching
    /// on every variant just to pull the string back out.
    pub fn message(&self) -> &str {
        match self {
            ApiError::NotFound(m)
            | ApiError::BadRequest(m)
            | ApiError::Conflict(m)
            | ApiError::Forbidden(m)
            | ApiError::Unauthorized(m)
            | ApiError::TooManyRequests(m) => m,
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            ApiError::NotFound(m) => (StatusCode::NOT_FOUND, m),
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ApiError::Conflict(m) => (StatusCode::CONFLICT, m),
            ApiError::Forbidden(m) => (StatusCode::FORBIDDEN, m),
            ApiError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, m),
            ApiError::TooManyRequests(m) => (StatusCode::TOO_MANY_REQUESTS, m),
        };
        (status, Json(json!({ "error": msg }))).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
