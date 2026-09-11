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
