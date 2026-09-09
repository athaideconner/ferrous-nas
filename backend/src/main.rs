//! FerrousNAS system daemon (`ferrous-nasd`).
//!
//! A mocked NAS control plane: it serves a JSON API describing disks, pools,
//! shares, apps, users and system stats, and optionally serves the built React
//! dashboard as static files. Nothing here performs real system operations —
//! every value is seeded and mutated in memory (see `state.rs`).

mod api;
mod app;
mod appmgr;
mod error;
mod models;
mod state;
mod telemetry;

use std::env;
use std::sync::Arc;

use axum::{routing::get, Router};
use tokio::sync::RwLock;
use tower_http::{
    cors::CorsLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing_subscriber::{prelude::*, EnvFilter};

use crate::app::AppState;
use crate::state::{Db, Store};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() {
    // Logging: honor RUST_LOG, default to info.
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=warn".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let db: Db = Arc::new(RwLock::new(Store::seeded()));
    let tel = telemetry::build(db.clone());
    let apps = appmgr::build(db.clone()).await;
    let app_state = AppState { db, tel, apps };

    let addr = env::var("FERROUS_ADDR").unwrap_or_else(|_| "0.0.0.0:4200".to_string());
    let web_dir = env::var("FERROUS_WEB_DIR").unwrap_or_else(|_| "../frontend/dist".to_string());

    // Serve the SPA: any unmatched path falls back to index.html so client-side
    // routing works. If the dist directory is absent (dev without a build),
    // these routes simply 404 and you use the Vite dev server instead.
    let index = format!("{web_dir}/index.html");
    let spa = ServeDir::new(&web_dir).not_found_service(ServeFile::new(index));

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .nest("/api/v1", api::router())
        .fallback_service(spa)
        // Dev convenience: the Vite dev server runs on another port, so allow
        // cross-origin calls. Tighten this for a real deployment.
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(app_state);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {addr}: {e}"));

    tracing::info!("FerrousNAS daemon v{VERSION} listening on http://{addr}");
    tracing::info!("API base: http://{addr}/api/v1  •  health: http://{addr}/healthz");

    axum::serve(listener, app)
        .await
        .expect("server error");
}
