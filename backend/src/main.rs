//! FerrousNAS system daemon (`ferrous-nasd`).
//!
//! A NAS control plane: it serves a JSON API describing disks, pools, shares,
//! apps, users and system stats, and optionally serves the built React
//! dashboard as static files.
//!
//! Every subsystem is **mocked by default** and can be independently switched
//! to a real implementation by environment variable (see README). Auth is the
//! exception: it is **on by default**, and disabling it restricts the daemon to
//! loopback.

mod api;
mod app;
mod appmgr;
mod auth;
mod error;
mod models;
mod poolmgr;
mod sharemgr;
mod state;
mod telemetry;

use std::env;
use std::net::SocketAddr;
use std::process::exit;
use std::sync::Arc;

use axum::{routing::get, Router};
use tokio::sync::RwLock;
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing_subscriber::{prelude::*, EnvFilter};

use crate::app::AppState;
use crate::auth::{AuthStore, AuthUser};
use crate::state::{Db, Store};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() {
    // Logging: honor RUST_LOG, default to info.
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=warn".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let addr = env::var("FERROUS_ADDR").unwrap_or_else(|_| "0.0.0.0:4200".to_string());
    let web_dir = env::var("FERROUS_WEB_DIR").unwrap_or_else(|_| "../frontend/dist".to_string());

    // Auth is opt-*out*, so the safe configuration is the one you get by doing
    // nothing at all.
    let auth_enabled = env::var("FERROUS_AUTH").as_deref() != Ok("off");
    if !auth_enabled && !is_loopback(&addr) {
        tracing::error!(
            "refusing to start: FERROUS_AUTH=off leaves the API unauthenticated, so it may only \
             be bound to loopback. Either remove FERROUS_AUTH=off, or set \
             FERROUS_ADDR=127.0.0.1:4200 (current: {addr})."
        );
        exit(1);
    }

    let db: Db = Arc::new(RwLock::new(Store::seeded()));

    // With auth disabled the user list is seeded from the mock store so the
    // Users page still works; nothing is written to disk in that mode.
    let seed: Vec<AuthUser> = if auth_enabled {
        Vec::new()
    } else {
        db.read()
            .await
            .users
            .iter()
            .map(|u| AuthUser {
                id: u.id.clone(),
                username: u.username.clone(),
                full_name: u.full_name.clone(),
                is_admin: u.is_admin,
                groups: u.groups.clone(),
                created_at: u.created_at.clone(),
                password_hash: None,
            })
            .collect()
    };

    // A failure to open the credential store is fatal — falling back to an
    // unauthenticated system would be exactly the wrong response.
    let auth = match AuthStore::load(auth_enabled, seed) {
        Ok(a) => Arc::new(a),
        Err(e) => {
            tracing::error!("refusing to start: cannot open the credential store: {e}");
            exit(1);
        }
    };

    if !auth.enabled {
        tracing::warn!("auth: DISABLED (loopback only) — every request runs as a local admin");
    } else if auth.setup_required().await {
        tracing::warn!("auth: no administrator configured — open the dashboard to run first-time setup");
    } else {
        tracing::info!("auth: enabled");
    }

    let tel = telemetry::build(db.clone());
    let apps = appmgr::build(db.clone()).await;
    let shares = sharemgr::build(db.clone());
    let pools = poolmgr::build(db.clone());
    let app_state = AppState { db, tel, apps, shares, pools, auth: auth.clone() };

    // Serve the SPA: any unmatched path falls back to index.html so client-side
    // routing works. If the dist directory is absent (dev without a build),
    // these routes simply 404 and you use the Vite dev server instead.
    let index = format!("{web_dir}/index.html");
    let spa = ServeDir::new(&web_dir).not_found_service(ServeFile::new(index));

    // No CORS layer: in production the daemon serves the dashboard itself, and
    // in development Vite proxies /api — both are same-origin. A permissive
    // policy would also be incompatible with credentialed cookies.
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .nest("/api/v1", api::router(auth))
        .fallback_service(spa)
        .layer(TraceLayer::new_for_http())
        .with_state(app_state);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {addr}: {e}"));

    tracing::info!("FerrousNAS daemon v{VERSION} listening on http://{addr}");
    tracing::info!("API base: http://{addr}/api/v1  •  health: http://{addr}/healthz");

    axum::serve(listener, app).await.expect("server error");
}

/// True when the bind address can only be reached from this machine.
fn is_loopback(addr: &str) -> bool {
    match addr.parse::<SocketAddr>() {
        Ok(s) => s.ip().is_loopback(),
        // Not a bare socket address (e.g. "localhost:4200") — accept only
        // names we are certain are local.
        Err(_) => {
            let host = addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(addr);
            matches!(host.trim_matches(['[', ']']), "localhost" | "127.0.0.1" | "::1")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_loopback;

    #[test]
    fn loopback_detection_gates_unauthenticated_mode() {
        for local in ["127.0.0.1:4200", "localhost:4200", "[::1]:4200", "127.0.0.5:80"] {
            assert!(is_loopback(local), "{local} should count as loopback");
        }
        for public in ["0.0.0.0:4200", "192.168.1.50:4200", "[::]:4200", "ferrous-nas.local:4200"] {
            assert!(!is_loopback(public), "{public} must NOT count as loopback");
        }
    }
}
