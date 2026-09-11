//! FerrousNAS system daemon (`ferrous-nasd`).
//!
//! A NAS control plane: it serves a JSON API describing disks, pools, shares,
//! apps, users and system stats, and optionally serves the built React
//! dashboard as static files.
//!
//! Every subsystem is **mocked by default** and can be independently switched
//! to a real implementation by environment variable (see README). Auth and TLS
//! are the exceptions: both are **on by default** — auth requires loopback to
//! disable, and TLS is self-signed out of the box rather than requiring a
//! reverse proxy.

mod api;
mod app;
mod appmgr;
mod auth;
mod error;
mod models;
mod poolmgr;
mod powermgr;
mod sharemgr;
mod state;
mod telemetry;
mod tls;
mod usermgr;

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
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

    // TLS is opt-out too: self-signed and generated on first boot unless a
    // real certificate is supplied, or this is explicitly turned off for a
    // reverse-proxy deployment.
    let tls_enabled = env::var("FERROUS_TLS").as_deref() != Ok("off");
    let tls_config = if tls_enabled {
        Some(load_tls_config().await.unwrap_or_else(|e| {
            tracing::error!("refusing to start: {e}");
            exit(1);
        }))
    } else {
        None
    };
    if tls_enabled {
        // Cookies really are going out over HTTPS now, so mark them Secure —
        // no manual step needed for the default path. `api::auth` reads this
        // per-request, so setting it once here before we start serving is
        // sufficient; it overrides any prior value because TLS being on makes
        // Secure unconditionally correct.
        env::set_var("FERROUS_COOKIE_SECURE", "1");
    } else if auth_enabled && !is_loopback(&addr) {
        tracing::warn!(
            "tls: DISABLED on a non-loopback address — the session cookie and every password \
             submission travel in clear text unless something in front of FerrousNAS terminates \
             TLS. If that's a reverse proxy, also set FERROUS_COOKIE_SECURE=1."
        );
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
    let power = powermgr::build();
    let user_ops = usermgr::build();
    let app_state = AppState { db, tel, apps, shares, pools, power, auth: auth.clone(), user_ops };

    // Serve the SPA: any unmatched path falls back to index.html so client-side
    // routing works. This must be `.fallback()`, not `.not_found_service()` —
    // the latter is tower-http's API for a custom *error* page and forces the
    // response to 404 regardless of what the fallback actually served; the
    // former passes the fallback's real status through, so a genuine
    // client-side route like /dashboard correctly reports 200.
    //
    // `/assets/*` (Vite's hashed JS/CSS output) is registered as its own
    // plain `ServeDir` with no fallback, ahead of the catch-all below, so a
    // genuinely missing asset still 404s for real rather than silently
    // getting the HTML shell back — which the browser would then fail to
    // parse as whatever content-type it expected. Everything else falls
    // through to the shell. If the dist directory is absent entirely (dev
    // without a build), index.html is then missing too, so the fallback
    // naturally 404s instead — use the Vite dev server.
    let index = format!("{web_dir}/index.html");
    let assets = ServeDir::new(format!("{web_dir}/assets"));
    let spa = ServeDir::new(&web_dir).fallback(ServeFile::new(index));

    // No CORS layer: in production the daemon serves the dashboard itself, and
    // in development Vite proxies /api — both are same-origin. A permissive
    // policy would also be incompatible with credentialed cookies.
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .nest("/api/v1", api::router(auth))
        .nest_service("/assets", assets)
        .fallback_service(spa)
        .layer(TraceLayer::new_for_http())
        .with_state(app_state);

    let scheme = if tls_enabled { "https" } else { "http" };
    tracing::info!("FerrousNAS daemon v{VERSION} listening on {scheme}://{addr}");
    tracing::info!("API base: {scheme}://{addr}/api/v1  •  health: {scheme}://{addr}/healthz");

    match tls_config {
        Some(config) => {
            let socket_addr = resolve_addr(&addr)
                .await
                .unwrap_or_else(|e| panic!("failed to resolve {addr}: {e}"));
            axum_server::bind_rustls(socket_addr, config)
                .serve(app.into_make_service())
                .await
                .expect("server error");
        }
        None => {
            let listener = tokio::net::TcpListener::bind(&addr)
                .await
                .unwrap_or_else(|e| panic!("failed to bind {addr}: {e}"));
            axum::serve(listener, app).await.expect("server error");
        }
    }
}

/// Directory the daemon keeps mutable state in — credentials, the self-signed
/// cert. Each consumer resolves it independently (matching how every other
/// subsystem reads its own env vars), so this one lives here rather than in
/// `auth`, which has its own copy for the same reason.
fn state_dir() -> PathBuf {
    PathBuf::from(env::var("FERROUS_STATE_DIR").unwrap_or_else(|_| "/var/lib/ferrous-nas".into()))
}

/// Resolve the cert/key pair to serve: a supplied real certificate if both
/// paths are set, otherwise a self-signed one generated into the state dir.
async fn load_tls_config() -> Result<axum_server::tls_rustls::RustlsConfig, String> {
    use axum_server::tls_rustls::RustlsConfig;

    let cert_env = env::var("FERROUS_TLS_CERT").ok();
    let key_env = env::var("FERROUS_TLS_KEY").ok();

    let (cert, key) = match (cert_env, key_env) {
        (Some(c), Some(k)) => {
            let (c, k) = (PathBuf::from(c), PathBuf::from(k));
            if !c.exists() {
                return Err(format!("FERROUS_TLS_CERT does not exist: {}", c.display()));
            }
            if !k.exists() {
                return Err(format!("FERROUS_TLS_KEY does not exist: {}", k.display()));
            }
            tracing::info!("tls: using supplied certificate {}", c.display());
            (c, k)
        }
        (None, None) => {
            let paths = tls::ensure_self_signed(&state_dir())?;
            (paths.cert, paths.key)
        }
        _ => {
            return Err(
                "FERROUS_TLS_CERT and FERROUS_TLS_KEY must both be set, or neither (to use a \
                 self-signed certificate)"
                    .to_string(),
            )
        }
    };

    RustlsConfig::from_pem_file(&cert, &key)
        .await
        .map_err(|e| format!("loading TLS certificate {}: {e}", cert.display()))
}

/// Resolve a `host:port` string (which may be a hostname, unlike
/// `SocketAddr::parse`) to a concrete socket address for `axum-server`.
async fn resolve_addr(addr: &str) -> Result<SocketAddr, String> {
    tokio::net::lookup_host(addr)
        .await
        .map_err(|e| e.to_string())?
        .next()
        .ok_or_else(|| format!("no address found for {addr}"))
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
