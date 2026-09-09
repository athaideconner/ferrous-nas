//! Users & groups.

use axum::{
    extract::{Path, State},
    Json,
};

use crate::error::{ApiError, ApiResult};
use crate::models::*;
use crate::state::Db;

pub async fn list_users(State(db): State<Db>) -> Json<Vec<User>> {
    Json(db.read().await.users.clone())
}

pub async fn list_groups(State(db): State<Db>) -> Json<Vec<Group>> {
    Json(db.read().await.groups.clone())
}

pub async fn create_user(
    State(db): State<Db>,
    Json(req): Json<CreateUserReq>,
) -> ApiResult<Json<User>> {
    if req.username.trim().is_empty() {
        return Err(ApiError::BadRequest("username is required".into()));
    }
    let mut store = db.write().await;
    if store.users.iter().any(|u| u.username == req.username) {
        return Err(ApiError::Conflict(format!("user '{}' already exists", req.username)));
    }
    let user = User {
        id: short_id("user"),
        username: req.username.clone(),
        full_name: req.full_name.clone(),
        is_admin: req.is_admin,
        groups: req.groups.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    store.users.push(user.clone());
    Ok(Json(user))
}

pub async fn delete_user(State(db): State<Db>, Path(id): Path<String>) -> ApiResult<Json<serde_json::Value>> {
    let mut store = db.write().await;
    let idx = store
        .users
        .iter()
        .position(|u| u.id == id)
        .ok_or_else(|| ApiError::NotFound(format!("user {id} not found")))?;
    if store.users[idx].username == "gorav" {
        return Err(ApiError::Conflict("cannot delete the primary admin".into()));
    }
    store.users.remove(idx);
    Ok(Json(serde_json::json!({ "ok": true })))
}
