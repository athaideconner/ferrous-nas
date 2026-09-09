//! Application state shared with every handler.
//!
//! Holds the mock datastore (`Db`) and the active telemetry source. `FromRef`
//! lets handlers keep extracting `State<Db>` as before, while telemetry-backed
//! handlers extract `State<TelemetryRef>` — both are derived from `AppState`.

use axum::extract::FromRef;

use crate::state::Db;
use crate::telemetry::TelemetryRef;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub tel: TelemetryRef,
}

impl FromRef<AppState> for Db {
    fn from_ref(s: &AppState) -> Db {
        s.db.clone()
    }
}

impl FromRef<AppState> for TelemetryRef {
    fn from_ref(s: &AppState) -> TelemetryRef {
        s.tel.clone()
    }
}
