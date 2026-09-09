//! The app "store" (catalog) plus installed container apps. Lifecycle actions
//! flip in-memory state instead of driving a real container runtime.

use axum::{
    extract::{Path, State},
    Json,
};
use rand::Rng;

use crate::error::{ApiError, ApiResult};
use crate::models::*;
use crate::state::Db;

pub async fn list_catalog(State(db): State<Db>) -> Json<Vec<CatalogApp>> {
    Json(db.read().await.catalog.clone())
}

pub async fn list_apps(State(db): State<Db>) -> Json<Vec<InstalledApp>> {
    Json(db.read().await.apps.clone())
}

pub async fn install_app(
    State(db): State<Db>,
    Json(req): Json<InstallAppReq>,
) -> ApiResult<Json<InstalledApp>> {
    let mut store = db.write().await;

    let cat = store
        .catalog
        .iter()
        .find(|c| c.id == req.catalog_id)
        .cloned()
        .ok_or_else(|| ApiError::NotFound(format!("catalog app {} not found", req.catalog_id)))?;

    if store.apps.iter().any(|a| a.catalog_id == cat.id) {
        return Err(ApiError::Conflict(format!("{} is already installed", cat.name)));
    }

    let host_port = req.host_port.unwrap_or(cat.default_port);
    if store.apps.iter().any(|a| a.host_port == host_port) {
        return Err(ApiError::Conflict(format!("port {host_port} is already in use")));
    }

    let app = InstalledApp {
        id: short_id("app"),
        catalog_id: cat.id.clone(),
        name: cat.name.clone(),
        icon: cat.icon.clone(),
        category: cat.category.clone(),
        image: cat.image.clone(),
        // A real daemon would go Installing -> Running; we jump straight to
        // Running so the mock UI is immediately useful.
        state: AppState::Running,
        host_port,
        cpu_percent: rand::thread_rng().gen_range(0.5..8.0),
        mem_bytes: rand::thread_rng().gen_range(80u64..600) * 1_000_000,
        web_ui: Some(format!("http://ferrous-nas.local:{host_port}")),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    store.apps.push(app.clone());
    Ok(Json(app))
}

pub async fn start_app(State(db): State<Db>, Path(id): Path<String>) -> ApiResult<Json<InstalledApp>> {
    set_state(db, id, AppState::Running).await
}

pub async fn stop_app(State(db): State<Db>, Path(id): Path<String>) -> ApiResult<Json<InstalledApp>> {
    set_state(db, id, AppState::Stopped).await
}

async fn set_state(db: Db, id: String, state: AppState) -> ApiResult<Json<InstalledApp>> {
    let mut store = db.write().await;
    let app = store
        .apps
        .iter_mut()
        .find(|a| a.id == id)
        .ok_or_else(|| ApiError::NotFound(format!("app {id} not found")))?;
    app.state = state;
    if state == AppState::Running {
        let mut rng = rand::thread_rng();
        app.cpu_percent = rng.gen_range(0.5..8.0);
        app.mem_bytes = rng.gen_range(80u64..600) * 1_000_000;
    } else {
        app.cpu_percent = 0.0;
        app.mem_bytes = 0;
    }
    Ok(Json(app.clone()))
}

pub async fn uninstall_app(State(db): State<Db>, Path(id): Path<String>) -> ApiResult<Json<serde_json::Value>> {
    let mut store = db.write().await;
    let idx = store
        .apps
        .iter()
        .position(|a| a.id == id)
        .ok_or_else(|| ApiError::NotFound(format!("app {id} not found")))?;
    store.apps.remove(idx);
    Ok(Json(serde_json::json!({ "ok": true })))
}
