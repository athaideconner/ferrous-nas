//! Public authentication endpoints: status, setup, login, logout — plus `me`,
//! which is protected.
//!
//! These live in the public router (no `require_auth` layer), so they must do
//! their own gating. `setup` is guarded by `setup_required`, which makes it
//! usable exactly once.

use axum::{
    extract::State,
    http::{header::SET_COOKIE, HeaderMap, HeaderValue},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::auth::middleware::CurrentUser;
use crate::auth::{clear_cookie, session_cookie, token_from_headers, AuthRef};
use crate::error::{ApiError, ApiResult};

#[derive(Deserialize)]
pub struct LoginReq {
    pub username: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct SetupReq {
    pub username: String,
    #[serde(default)]
    pub full_name: String,
    pub password: String,
}

/// Unauthenticated: lets the dashboard decide between the setup screen, the
/// login screen, or going straight in.
pub async fn status(State(auth): State<AuthRef>) -> Json<serde_json::Value> {
    Json(json!({
        "auth_enabled": auth.enabled,
        "setup_required": auth.setup_required().await,
    }))
}

pub async fn setup(
    State(auth): State<AuthRef>,
    Json(req): Json<SetupReq>,
) -> ApiResult<impl IntoResponse> {
    let full_name = if req.full_name.trim().is_empty() {
        req.username.clone()
    } else {
        req.full_name.clone()
    };
    let (user, token) = auth
        .setup_first_admin(&req.username, &full_name, &req.password)
        .await?;
    tracing::info!("auth: initial administrator '{}' created", user.username);
    Ok((cookie_headers(&session_cookie(&token))?, Json(user)))
}

pub async fn login(
    State(auth): State<AuthRef>,
    Json(req): Json<LoginReq>,
) -> ApiResult<impl IntoResponse> {
    let (user, token) = auth.login(&req.username, &req.password).await?;
    tracing::info!("auth: '{}' signed in", user.username);
    Ok((cookie_headers(&session_cookie(&token))?, Json(user)))
}

pub async fn logout(State(auth): State<AuthRef>, headers: HeaderMap) -> ApiResult<impl IntoResponse> {
    if let Some(token) = token_from_headers(&headers) {
        auth.logout(&token).await;
    }
    // Always clear the cookie, even if there was no valid session.
    Ok((cookie_headers(&clear_cookie())?, Json(json!({ "ok": true }))))
}

/// Protected: who am I?
pub async fn me(CurrentUser(user): CurrentUser) -> Json<crate::models::User> {
    Json(user.to_public())
}

fn cookie_headers(cookie: &str) -> ApiResult<HeaderMap> {
    let mut headers = HeaderMap::new();
    let value = HeaderValue::from_str(cookie)
        .map_err(|_| ApiError::BadRequest("invalid session cookie".into()))?;
    headers.insert(SET_COOKIE, value);
    Ok(headers)
}
