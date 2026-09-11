//! Users & groups.
//!
//! Users live in the [`AuthStore`](crate::auth::AuthStore), which owns their
//! credentials — so creating a user here creates a real login. Groups are still
//! mock data.

use axum::{
    extract::{Path, State},
    Json,
};

use crate::auth::middleware::AdminUser;
use crate::auth::AuthRef;
use crate::error::{ApiError, ApiResult};
use crate::models::*;
use crate::state::Db;

pub async fn list_users(State(auth): State<AuthRef>) -> Json<Vec<User>> {
    Json(auth.list_users().await)
}

pub async fn list_groups(State(db): State<Db>) -> Json<Vec<Group>> {
    Json(db.read().await.groups.clone())
}

pub async fn create_user(
    _admin: AdminUser,
    State(auth): State<AuthRef>,
    Json(req): Json<CreateUserReq>,
) -> ApiResult<Json<User>> {
    let full_name = if req.full_name.trim().is_empty() {
        req.username.clone()
    } else {
        req.full_name.clone()
    };
    let password = req.password.as_deref().filter(|p| !p.is_empty());
    let user = auth
        .create_user(&req.username, &full_name, req.is_admin, req.groups.clone(), password)
        .await?;
    Ok(Json(user))
}

pub async fn delete_user(
    AdminUser(actor): AdminUser,
    State(auth): State<AuthRef>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    // Deleting yourself would end your own session mid-request; make it an
    // explicit refusal rather than a confusing logout.
    if actor.id == id {
        return Err(ApiError::Conflict(
            "you cannot delete your own account".into(),
        ));
    }
    auth.delete_user(&id).await?;
    tracing::info!("users: '{}' deleted user {id}", actor.username);
    Ok(Json(serde_json::json!({ "ok": true })))
}
