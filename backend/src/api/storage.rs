//! Disks, pools and datasets. Create/delete mutate the in-memory store so the
//! UI feels live, but nothing touches a real block device.

use axum::{
    extract::{Path, State},
    Json,
};

use crate::error::{ApiError, ApiResult};
use crate::models::*;
use crate::state::Db;
use crate::telemetry::TelemetryRef;

const GB: u64 = 1_000_000_000;

/// Physical disks come from the telemetry source: the mock store by default,
/// or real `lsblk`/`smartctl` output when `FERROUS_TELEMETRY=linux`.
pub async fn list_disks(State(tel): State<TelemetryRef>) -> Json<Vec<Disk>> {
    Json(tel.disks().await)
}

pub async fn list_pools(State(db): State<Db>) -> Json<Vec<Pool>> {
    Json(db.read().await.pools.clone())
}

pub async fn create_pool(
    State(db): State<Db>,
    Json(req): Json<CreatePoolReq>,
) -> ApiResult<Json<Pool>> {
    if req.name.trim().is_empty() {
        return Err(ApiError::BadRequest("pool name is required".into()));
    }
    let min_disks = match req.raid_level {
        RaidLevel::Stripe => 1,
        RaidLevel::Mirror => 2,
        RaidLevel::Raidz1 => 3,
        RaidLevel::Raidz2 => 4,
    };
    if req.disk_ids.len() < min_disks {
        return Err(ApiError::BadRequest(format!(
            "this RAID level needs at least {min_disks} disk(s)"
        )));
    }

    let mut store = db.write().await;

    if store.pools.iter().any(|p| p.name == req.name) {
        return Err(ApiError::Conflict(format!("pool '{}' already exists", req.name)));
    }

    // Validate disks exist and are free; compute raw capacity.
    let mut raw = 0u64;
    for id in &req.disk_ids {
        let disk = store
            .disks
            .iter()
            .find(|d| &d.id == id)
            .ok_or_else(|| ApiError::BadRequest(format!("unknown disk {id}")))?;
        if disk.pool_id.is_some() {
            return Err(ApiError::Conflict(format!("disk {} is already in a pool", disk.device)));
        }
        raw += disk.size_bytes;
    }

    // Usable capacity heuristics (mocked, roughly like ZFS).
    let n = req.disk_ids.len() as u64;
    let usable = match req.raid_level {
        RaidLevel::Stripe => raw,
        RaidLevel::Mirror => raw / n,
        RaidLevel::Raidz1 => raw * (n - 1) / n,
        RaidLevel::Raidz2 => raw * (n - 2) / n,
    };

    let pool = Pool {
        id: short_id("pool"),
        name: req.name.clone(),
        raid_level: req.raid_level,
        status: PoolStatus::Online,
        size_bytes: usable,
        used_bytes: 0,
        disk_ids: req.disk_ids.clone(),
        scrub_progress: None,
    };

    // Mark the disks as claimed.
    for id in &req.disk_ids {
        if let Some(d) = store.disks.iter_mut().find(|d| &d.id == id) {
            d.pool_id = Some(pool.id.clone());
        }
    }

    store.pools.push(pool.clone());
    Ok(Json(pool))
}

pub async fn delete_pool(State(db): State<Db>, Path(id): Path<String>) -> ApiResult<Json<serde_json::Value>> {
    let mut store = db.write().await;

    let idx = store
        .pools
        .iter()
        .position(|p| p.id == id)
        .ok_or_else(|| ApiError::NotFound(format!("pool {id} not found")))?;

    if store.datasets.iter().any(|d| d.pool_id == id) {
        return Err(ApiError::Conflict(
            "pool still has datasets; delete them first".into(),
        ));
    }

    // Release its disks.
    for d in store.disks.iter_mut() {
        if d.pool_id.as_deref() == Some(id.as_str()) {
            d.pool_id = None;
        }
    }
    store.pools.remove(idx);
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn scrub_pool(State(db): State<Db>, Path(id): Path<String>) -> ApiResult<Json<Pool>> {
    let mut store = db.write().await;
    let pool = store
        .pools
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or_else(|| ApiError::NotFound(format!("pool {id} not found")))?;
    pool.status = PoolStatus::Scrubbing;
    pool.scrub_progress = Some(0.0);
    Ok(Json(pool.clone()))
}

pub async fn list_datasets(State(db): State<Db>) -> Json<Vec<Dataset>> {
    Json(db.read().await.datasets.clone())
}

pub async fn create_dataset(
    State(db): State<Db>,
    Json(req): Json<CreateDatasetReq>,
) -> ApiResult<Json<Dataset>> {
    if req.name.trim().is_empty() {
        return Err(ApiError::BadRequest("dataset name is required".into()));
    }
    let mut store = db.write().await;

    let pool = store
        .pools
        .iter()
        .find(|p| p.id == req.pool_id)
        .ok_or_else(|| ApiError::BadRequest(format!("unknown pool {}", req.pool_id)))?;
    let pool_name = pool.name.clone();

    if store
        .datasets
        .iter()
        .any(|d| d.pool_id == req.pool_id && d.name == req.name)
    {
        return Err(ApiError::Conflict(format!(
            "dataset '{}' already exists in this pool",
            req.name
        )));
    }

    let ds = Dataset {
        id: short_id("ds"),
        pool_id: req.pool_id.clone(),
        name: req.name.clone(),
        path: format!("/mnt/{pool_name}/{}", req.name),
        used_bytes: 0,
        quota_bytes: req.quota_gb.map(|g| g * GB),
        compression: req.compression,
    };
    store.datasets.push(ds.clone());
    Ok(Json(ds))
}

pub async fn delete_dataset(State(db): State<Db>, Path(id): Path<String>) -> ApiResult<Json<serde_json::Value>> {
    let mut store = db.write().await;

    if store.shares.iter().any(|s| s.dataset_id == id) {
        return Err(ApiError::Conflict(
            "dataset is used by a share; delete the share first".into(),
        ));
    }
    let idx = store
        .datasets
        .iter()
        .position(|d| d.id == id)
        .ok_or_else(|| ApiError::NotFound(format!("dataset {id} not found")))?;
    store.datasets.remove(idx);
    Ok(Json(serde_json::json!({ "ok": true })))
}
