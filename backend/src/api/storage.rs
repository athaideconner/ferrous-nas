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
use crate::error::ApiError;
use crate::poolmgr::PoolManagerRef;
use crate::sharemgr::ShareManagerRef;
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

/// Refuse to destroy storage a share still serves: the share would point at
/// nothing, and whoever relies on it would lose it without warning.
async fn refuse_if_shared(shares: &ShareManagerRef, what: &str, covers: impl Fn(&str) -> bool) -> ApiResult<()> {
    let names: Vec<String> = shares
        .list()
        .await?
        .into_iter()
        .filter(|s| covers(&s.dataset_id))
        .map(|s| s.name)
        .collect();
    if names.is_empty() {
        return Ok(());
    }
    Err(ApiError::Conflict(format!("{what} is shared as {} — delete the share first", names.join(", "))))
}

pub async fn delete_pool(
    _admin: AdminUser,
    State(mgr): State<PoolManagerRef>,
    State(shares): State<ShareManagerRef>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    refuse_if_shared(&shares, &format!("Pool {id}"), |ds| ds.split('/').next() == Some(id.as_str())).await?;
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
    State(shares): State<ShareManagerRef>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    refuse_if_shared(&shares, &format!("Dataset {id}"), |ds| ds == id).await?;
    mgr.delete_dataset(&id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}
