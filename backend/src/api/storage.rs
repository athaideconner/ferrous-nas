//! Disks, pools and datasets.
//!
//! Disks come from the telemetry source; pools and datasets from the active
//! [`PoolManager`](crate::poolmgr) — the mock store by default, or real
//! `zpool`/`zfs` when `FERROUS_POOLS=zfs`.

use axum::{
    extract::{Path, State},
    Json,
};

use crate::auth::middleware::AdminUser;
use crate::error::ApiResult;
use crate::models::*;
use crate::poolmgr::PoolManagerRef;
use crate::telemetry::TelemetryRef;

/// Physical disks come from the telemetry source: the mock store by default,
/// or real `lsblk`/`smartctl` output when `FERROUS_TELEMETRY=linux`.
pub async fn list_disks(State(tel): State<TelemetryRef>) -> Json<Vec<Disk>> {
    Json(tel.disks().await)
}

pub async fn list_pools(State(mgr): State<PoolManagerRef>) -> ApiResult<Json<Vec<Pool>>> {
    Ok(Json(mgr.pools().await?))
}

pub async fn create_pool(
    _admin: AdminUser,
    State(mgr): State<PoolManagerRef>,
    Json(req): Json<CreatePoolReq>,
) -> ApiResult<Json<Pool>> {
    Ok(Json(mgr.create_pool(req).await?))
}

pub async fn delete_pool(
    _admin: AdminUser,
    State(mgr): State<PoolManagerRef>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    mgr.delete_pool(&id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn scrub_pool(
    _admin: AdminUser,
    State(mgr): State<PoolManagerRef>,
    Path(id): Path<String>,
) -> ApiResult<Json<Pool>> {
    Ok(Json(mgr.scrub_pool(&id).await?))
}

pub async fn list_datasets(State(mgr): State<PoolManagerRef>) -> ApiResult<Json<Vec<Dataset>>> {
    Ok(Json(mgr.datasets().await?))
}

pub async fn create_dataset(
    _admin: AdminUser,
    State(mgr): State<PoolManagerRef>,
    Json(req): Json<CreateDatasetReq>,
) -> ApiResult<Json<Dataset>> {
    Ok(Json(mgr.create_dataset(req).await?))
}

pub async fn delete_dataset(
    _admin: AdminUser,
    State(mgr): State<PoolManagerRef>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    mgr.delete_dataset(&id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}
