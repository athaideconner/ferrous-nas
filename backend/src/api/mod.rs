//! HTTP API surface. Every route is versioned under `/api/v1`.

pub mod apps;
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

/// Build the full `/api/v1` router.
pub fn router() -> Router<AppState> {
    Router::new()
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
        .route("/groups", get(users::list_groups))
        // network
        .route("/network/interfaces", get(network::list_interfaces))
}
