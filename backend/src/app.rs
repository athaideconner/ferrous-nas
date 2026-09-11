//! Application state shared with every handler.
//!
//! Holds the mock datastore (`Db`) and the active telemetry source. `FromRef`
//! lets handlers keep extracting `State<Db>` as before, while telemetry-backed
//! handlers extract `State<TelemetryRef>` — both are derived from `AppState`.

use axum::extract::FromRef;

use crate::appmgr::AppManagerRef;
use crate::auth::AuthRef;
use crate::poolmgr::PoolManagerRef;
use crate::powermgr::PowerManagerRef;
use crate::sharemgr::ShareManagerRef;
use crate::state::Db;
use crate::telemetry::TelemetryRef;
use crate::usermgr::UserOpsRef;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub tel: TelemetryRef,
    pub apps: AppManagerRef,
    pub shares: ShareManagerRef,
    pub pools: PoolManagerRef,
    pub power: PowerManagerRef,
    pub auth: AuthRef,
    pub user_ops: UserOpsRef,
}

impl FromRef<AppState> for AuthRef {
    fn from_ref(s: &AppState) -> AuthRef {
        s.auth.clone()
    }
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

impl FromRef<AppState> for AppManagerRef {
    fn from_ref(s: &AppState) -> AppManagerRef {
        s.apps.clone()
    }
}

impl FromRef<AppState> for ShareManagerRef {
    fn from_ref(s: &AppState) -> ShareManagerRef {
        s.shares.clone()
    }
}

impl FromRef<AppState> for PoolManagerRef {
    fn from_ref(s: &AppState) -> PoolManagerRef {
        s.pools.clone()
    }
}

impl FromRef<AppState> for PowerManagerRef {
    fn from_ref(s: &AppState) -> PowerManagerRef {
        s.power.clone()
    }
}

impl FromRef<AppState> for UserOpsRef {
    fn from_ref(s: &AppState) -> UserOpsRef {
        s.user_ops.clone()
    }
}
