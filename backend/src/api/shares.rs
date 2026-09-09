//! SMB / NFS shares.

use axum::{
    extract::{Path, State},
    Json,
};

use crate::error::{ApiError, ApiResult};
use crate::models::*;
use crate::state::Db;

pub async fn list_shares(State(db): State<Db>) -> Json<Vec<Share>> {
    Json(db.read().await.shares.clone())
}

pub async fn create_share(
    State(db): State<Db>,
    Json(req): Json<CreateShareReq>,
) -> ApiResult<Json<Share>> {
    if req.name.trim().is_empty() {
        return Err(ApiError::BadRequest("share name is required".into()));
    }
    let mut store = db.write().await;

    let dataset = store
        .datasets
        .iter()
        .find(|d| d.id == req.dataset_id)
        .ok_or_else(|| ApiError::BadRequest(format!("unknown dataset {}", req.dataset_id)))?;
    let path = dataset.path.clone();

    if store.shares.iter().any(|s| s.name == req.name) {
        return Err(ApiError::Conflict(format!("share '{}' already exists", req.name)));
    }

    let share = Share {
        id: short_id("share"),
        name: req.name.clone(),
        kind: req.kind,
        dataset_id: req.dataset_id.clone(),
        path,
        enabled: true,
        read_only: req.read_only,
        guest_ok: req.guest_ok,
        allowed_users: req.allowed_users.clone(),
    };
    store.shares.push(share.clone());
    Ok(Json(share))
}

pub async fn patch_share(
    State(db): State<Db>,
    Path(id): Path<String>,
    Json(req): Json<PatchShareReq>,
) -> ApiResult<Json<Share>> {
    let mut store = db.write().await;
    let share = store
        .shares
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or_else(|| ApiError::NotFound(format!("share {id} not found")))?;
    if let Some(v) = req.enabled {
        share.enabled = v;
    }
    if let Some(v) = req.read_only {
        share.read_only = v;
    }
    if let Some(v) = req.guest_ok {
        share.guest_ok = v;
    }
    Ok(Json(share.clone()))
}

pub async fn delete_share(State(db): State<Db>, Path(id): Path<String>) -> ApiResult<Json<serde_json::Value>> {
    let mut store = db.write().await;
    let idx = store
        .shares
        .iter()
        .position(|s| s.id == id)
        .ok_or_else(|| ApiError::NotFound(format!("share {id} not found")))?;
    store.shares.remove(idx);
    Ok(Json(serde_json::json!({ "ok": true })))
}
