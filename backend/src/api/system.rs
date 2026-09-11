//! System info, live stats, power actions, and alerts. All mocked.

use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::middleware::AdminUser;
use crate::error::{ApiError, ApiResult};
use crate::models::{Alert, StatPoint, SystemInfo};
use crate::state::Db;
use crate::telemetry::TelemetryRef;

pub async fn get_system(State(tel): State<TelemetryRef>) -> Json<SystemInfo> {
    Json(tel.system_info().await)
}

/// Reports which telemetry source is active ("mock" or "linux").
pub async fn get_telemetry_source(State(tel): State<TelemetryRef>) -> Json<Value> {
    Json(json!({ "source": tel.source() }))
}

#[derive(Deserialize)]
pub struct StatsQuery {
    #[serde(default = "default_points")]
    points: usize,
}
fn default_points() -> usize {
    60
}

pub async fn get_stats(State(tel): State<TelemetryRef>, Query(q): Query<StatsQuery>) -> Json<Vec<StatPoint>> {
    let points = q.points.clamp(2, 240);
    Json(tel.stats_history(points).await)
}

pub async fn reboot(_admin: AdminUser) -> Json<Value> {
    // Mocked: we never actually reboot. Return what a real daemon would ack.
    Json(json!({ "ok": true, "action": "reboot", "note": "mock — no action taken" }))
}

pub async fn shutdown(_admin: AdminUser) -> Json<Value> {
    Json(json!({ "ok": true, "action": "shutdown", "note": "mock — no action taken" }))
}

pub async fn list_alerts(State(db): State<Db>) -> Json<Vec<Alert>> {
    Json(db.read().await.alerts.clone())
}

pub async fn ack_alert(
    _admin: AdminUser,State(db): State<Db>, Path(id): Path<String>) -> ApiResult<Json<Alert>> {
    let mut store = db.write().await;
    let alert = store
        .alerts
        .iter_mut()
        .find(|a| a.id == id)
        .ok_or_else(|| ApiError::NotFound(format!("alert {id} not found")))?;
    alert.acknowledged = true;
    Ok(Json(alert.clone()))
}
