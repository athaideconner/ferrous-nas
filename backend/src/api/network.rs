//! Network interfaces (read-only mock).

use axum::{extract::State, Json};

use crate::models::NetInterface;
use crate::state::Db;

pub async fn list_interfaces(State(db): State<Db>) -> Json<Vec<NetInterface>> {
    Json(db.read().await.interfaces.clone())
}
