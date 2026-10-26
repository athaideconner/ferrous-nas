//! Network interfaces and settings, from the telemetry source: the seeded
//! sample data by default, or this machine's real network when
//! `FERROUS_TELEMETRY=linux`. Read-only.

use axum::{extract::State, Json};

use crate::models::{NetInterface, NetworkSummary};
use crate::telemetry::TelemetryRef;

pub async fn list_interfaces(State(tel): State<TelemetryRef>) -> Json<Vec<NetInterface>> {
    Json(tel.interfaces().await)
}

pub async fn summary(State(tel): State<TelemetryRef>) -> Json<NetworkSummary> {
    Json(tel.network_summary().await)
}
