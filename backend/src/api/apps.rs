//! The app "store" (catalog) plus installed apps. Lifecycle actions are handled
//! by the active [`AppManager`](crate::appmgr) — the mock store by default, or
//! the real Docker backend when `FERROUS_APPS=docker`.

use axum::{
    extract::{Path, State},
    Json,
};

use crate::appmgr::AppManagerRef;
use crate::error::{ApiError, ApiResult};
use crate::models::*;
use crate::state::Db;

pub async fn list_catalog(State(db): State<Db>) -> Json<Vec<CatalogApp>> {
    // The catalog is static data; it lives in the store regardless of backend.
    Json(db.read().await.catalog.clone())
}

pub async fn list_apps(State(mgr): State<AppManagerRef>) -> ApiResult<Json<Vec<InstalledApp>>> {
    Ok(Json(mgr.list().await?))
}

pub async fn install_app(
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

pub async fn start_app(State(mgr): State<AppManagerRef>, Path(id): Path<String>) -> ApiResult<Json<InstalledApp>> {
    Ok(Json(mgr.start(&id).await?))
}

pub async fn stop_app(State(mgr): State<AppManagerRef>, Path(id): Path<String>) -> ApiResult<Json<InstalledApp>> {
    Ok(Json(mgr.stop(&id).await?))
}

pub async fn uninstall_app(State(mgr): State<AppManagerRef>, Path(id): Path<String>) -> ApiResult<Json<serde_json::Value>> {
    mgr.uninstall(&id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}
