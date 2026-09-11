//! Session enforcement.
//!
//! [`require_auth`] is layered onto the *protected* router only — the public
//! auth/setup routes live in a separate router that never has this layer. That
//! structural split is deliberate: there is no path allowlist to get wrong, so
//! a new route cannot accidentally become public by matching a prefix.
//!
//! The middleware only *authenticates* (resolving the session and stashing the
//! user in request extensions). *Authorisation* is per-handler via the
//! [`AdminUser`] extractor, so every privileged endpoint states its own
//! requirement in its signature.

use async_trait::async_trait;
use axum::extract::{FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::{token_from_headers, AuthRef, AuthUser};
use crate::error::ApiError;

/// Stand-in identity used when auth is disabled (loopback-only dev mode).
fn local_admin() -> AuthUser {
    AuthUser {
        id: "user-local".into(),
        username: "local".into(),
        full_name: "Local (auth disabled)".into(),
        is_admin: true,
        groups: vec!["admins".into()],
        created_at: String::new(),
        password_hash: None,
    }
}

pub async fn require_auth(State(auth): State<AuthRef>, mut req: Request, next: Next) -> Response {
    if !auth.enabled {
        req.extensions_mut().insert(local_admin());
        return next.run(req).await;
    }

    let user = match token_from_headers(req.headers()) {
        Some(token) => auth.user_for_token(&token).await,
        None => None,
    };

    match user {
        Some(u) => {
            req.extensions_mut().insert(u);
            next.run(req).await
        }
        None => ApiError::Unauthorized("authentication required".into()).into_response(),
    }
}

/// Any authenticated user.
pub struct CurrentUser(pub AuthUser);

/// An authenticated user who is an administrator. Handlers that mutate system
/// state take this, so the requirement is visible in the signature.
pub struct AdminUser(pub AuthUser);

#[async_trait]
impl<S: Send + Sync> FromRequestParts<S> for CurrentUser {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthUser>()
            .cloned()
            .map(CurrentUser)
            .ok_or_else(|| ApiError::Unauthorized("authentication required".into()))
    }
}

#[async_trait]
impl<S: Send + Sync> FromRequestParts<S> for AdminUser {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let user = parts
            .extensions
            .get::<AuthUser>()
            .cloned()
            .ok_or_else(|| ApiError::Unauthorized("authentication required".into()))?;
        if !user.is_admin {
            return Err(ApiError::Forbidden(
                "administrator privileges are required for this operation".into(),
            ));
        }
        Ok(AdminUser(user))
    }
}
