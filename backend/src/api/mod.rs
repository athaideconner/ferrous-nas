//! HTTP API surface. Every route is versioned under `/api/v1`.
//!
//! The router is split in two:
//!
//! * **public** — status/setup/login/logout. No session required.
//! * **protected** — everything else, behind [`require_auth`].
//!
//! Splitting by router rather than by a path allowlist means a newly added
//! route is protected by construction; you have to deliberately put something
//! in the public router for it to be reachable without a session.

pub mod apps;
pub mod auth;
pub mod network;
pub mod shares;
pub mod storage;
pub mod system;
pub mod users;

use axum::{
    routing::{delete, get, patch, post},
    Router,
};

use crate::app::AppState;
use crate::auth::middleware::require_auth;
use crate::auth::AuthRef;

/// Build the full `/api/v1` router. `auth` is needed to construct the session
/// middleware layer.
pub fn router(auth: AuthRef) -> Router<AppState> {
    public().merge(protected(auth))
}

/// Reachable without a session.
fn public() -> Router<AppState> {
    Router::new()
        .route("/auth/status", get(auth::status))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/setup", post(auth::setup))
}

/// Requires a valid session (or auth disabled). Mutating handlers additionally
/// require `AdminUser`.
fn protected(auth: AuthRef) -> Router<AppState> {
    Router::new()
        .route("/auth/me", get(auth::me))
        // system
        .route("/system", get(system::get_system))
        .route("/system/stats", get(system::get_stats))
        .route("/system/telemetry", get(system::get_telemetry_source))
        .route("/system/reboot", post(system::reboot))
        .route("/system/shutdown", post(system::shutdown))
        .route("/alerts", get(system::list_alerts))
        .route("/alerts/:id/ack", post(system::ack_alert))
        // storage
        .route("/storage/disks", get(storage::list_disks))
        .route("/storage/pools", get(storage::list_pools).post(storage::create_pool))
        .route("/storage/pools/:id", delete(storage::delete_pool))
        .route("/storage/pools/:id/scrub", post(storage::scrub_pool))
        .route("/storage/datasets", get(storage::list_datasets).post(storage::create_dataset))
        .route("/storage/datasets/:id", delete(storage::delete_dataset))
        // shares
        .route("/shares", get(shares::list_shares).post(shares::create_share))
        .route("/shares/:id", patch(shares::patch_share).delete(shares::delete_share))
        // apps
        .route("/apps/catalog", get(apps::list_catalog))
        .route("/apps", get(apps::list_apps).post(apps::install_app))
        .route("/apps/:id", delete(apps::uninstall_app))
        .route("/apps/:id/start", post(apps::start_app))
        .route("/apps/:id/stop", post(apps::stop_app))
        // users & groups
        .route("/users", get(users::list_users).post(users::create_user))
        .route("/users/:id", delete(users::delete_user))
        .route("/groups", get(users::list_groups).post(users::create_group))
        .route("/groups/:id", delete(users::delete_group))
        // network
        .route("/network/interfaces", get(network::list_interfaces))
        .layer(axum::middleware::from_fn_with_state(auth, require_auth))
}
