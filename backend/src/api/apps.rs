//! The app "store" (catalog) plus installed apps. Lifecycle actions are handled
//! by the active [`AppManager`](crate::appmgr) — the mock store by default, or
//! the real Docker backend when `FERROUS_APPS=docker`.

use axum::{
    extract::{Path, State},
    Json,
};

use crate::appmgr::recipes::DEFAULT_FILES_ROOT;
use crate::appmgr::AppManagerRef;
use crate::auth::middleware::{AdminUser, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::models::*;
use crate::poolmgr::PoolManagerRef;
use crate::state::Db;

pub async fn list_catalog(State(db): State<Db>) -> Json<Vec<CatalogApp>> {
    // The catalog is static data; it lives in the store regardless of backend.
    Json(db.read().await.catalog.clone())
}

pub async fn list_apps(
    CurrentUser(user): CurrentUser,
    State(mgr): State<AppManagerRef>,
) -> ApiResult<Json<Vec<InstalledApp>>> {
    let mut apps = mgr.list().await?;
    if !user.is_admin {
        // Hints can carry an app's generated admin password.
        apps.iter_mut().for_each(|a| a.hint = None);
    }
    Ok(Json(apps))
}

pub async fn install_app(
    _admin: AdminUser,
    State(db): State<Db>,
    State(mgr): State<AppManagerRef>,
    Json(req): Json<InstallAppReq>,
) -> ApiResult<Json<InstalledApp>> {
    // Resolve the catalog entry (from the static store), then hand off to the
    // backend, which owns the actual install.
    let cat = db
        .read()
        .await
        .catalog
        .iter()
        .find(|c| c.id == req.catalog_id)
        .cloned()
        .ok_or_else(|| ApiError::NotFound(format!("catalog app {} not found", req.catalog_id)))?;
    let host_port = req.host_port.unwrap_or(cat.default_port);
    Ok(Json(mgr.install(&cat, host_port).await?))
}

pub async fn start_app(
    _admin: AdminUser,State(mgr): State<AppManagerRef>, Path(id): Path<String>) -> ApiResult<Json<InstalledApp>> {
    Ok(Json(mgr.start(&id).await?))
}

pub async fn stop_app(
    _admin: AdminUser,State(mgr): State<AppManagerRef>, Path(id): Path<String>) -> ApiResult<Json<InstalledApp>> {
    Ok(Json(mgr.stop(&id).await?))
}

pub async fn uninstall_app(
    _admin: AdminUser,State(mgr): State<AppManagerRef>, Path(id): Path<String>) -> ApiResult<Json<serde_json::Value>> {
    mgr.uninstall(&id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Where apps keep files people work with, and where they could: the
/// system disk, or any pool dataset.
pub async fn get_files_location(
    State(mgr): State<AppManagerRef>,
    State(pools): State<PoolManagerRef>,
) -> ApiResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({
        "files_root": mgr.files_root().await,
        "options": files_options(&pools).await?,
    })))
}

#[derive(serde::Deserialize)]
pub struct FilesLocationReq {
    pub files_root: String,
}

pub async fn set_files_location(
    _admin: AdminUser,
    State(db): State<Db>,
    State(mgr): State<AppManagerRef>,
    State(pools): State<PoolManagerRef>,
    Json(req): Json<FilesLocationReq>,
) -> ApiResult<Json<serde_json::Value>> {
    // Only somewhere offered: never an arbitrary path on the system.
    let options = files_options(&pools).await?;
    if !options.iter().any(|o| o["path"] == req.files_root.as_str()) {
        return Err(ApiError::BadRequest(format!("{} is not a dataset or the system disk", req.files_root)));
    }
    let catalog = db.read().await.catalog.clone();
    mgr.set_files_root(&req.files_root, &catalog).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn files_options(pools: &PoolManagerRef) -> ApiResult<Vec<serde_json::Value>> {
    let mut options = vec![serde_json::json!({ "label": "System disk", "path": DEFAULT_FILES_ROOT })];
    for d in pools.datasets().await? {
        let label = format!("{}/{}", d.pool_id, d.name);
        options.push(serde_json::json!({ "label": label, "path": d.path }));
    }
    Ok(options)
}
