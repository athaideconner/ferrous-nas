//! App management subsystem.
//!
//! Installed apps are served through the [`AppManager`] trait, with two
//! implementations chosen by env var:
//!
//! - [`MockAppManager`] — the default. Manipulates the seeded in-memory store.
//! - [`docker::DockerAppManager`] — drives the real Docker Engine over its unix
//!   socket (`/var/run/docker.sock`): pull, create, start, stop, remove.
//!
//! ```text
//! FERROUS_APPS=docker   # or "real" — manage real containers
//! (unset / anything else)   # mocked
//! ```
//!
//! The app *catalog* (the store listing) is static data and is always served
//! from the seeded store regardless of backend. Only the installed-app
//! lifecycle is backend-specific.

pub mod docker;
pub mod recipes;

use std::sync::Arc;

use async_trait::async_trait;
use rand::Rng;

use crate::error::{ApiError, ApiResult};
use crate::models::*;
use crate::state::Db;

pub type AppManagerRef = Arc<dyn AppManager>;

#[async_trait]
pub trait AppManager: Send + Sync {
    fn source(&self) -> &'static str;
    async fn list(&self) -> ApiResult<Vec<InstalledApp>>;
    /// `cat` is the resolved catalog entry; `host_port` is already defaulted.
    async fn install(&self, cat: &CatalogApp, host_port: u16) -> ApiResult<InstalledApp>;
    async fn start(&self, id: &str) -> ApiResult<InstalledApp>;
    async fn stop(&self, id: &str) -> ApiResult<InstalledApp>;
    async fn uninstall(&self, id: &str) -> ApiResult<()>;
    /// Where apps keep files people work with (media, downloads, photos,
    /// scans) — see [`recipes::DEFAULT_FILES_ROOT`].
    async fn files_root(&self) -> String;
    /// Move that to `root` (already validated by the caller), recreating the
    /// installed apps that use it. `catalog` describes those apps.
    async fn set_files_root(&self, root: &str, catalog: &[CatalogApp]) -> ApiResult<()>;
}

/// Create `root/dir` (and the folders between) as shared folders: group
/// `users`, group-writable, setgid — like shared folders (see sharemgr), so
/// apps running as a user can write them and SMB/NFS users can manage them.
pub(crate) fn prepare_files_dir(root: &str, dir: &str) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut path = std::path::PathBuf::from(root);
    std::fs::create_dir_all(&path)?;
    let prepare = |p: &std::path::Path| -> std::io::Result<()> {
        std::os::unix::fs::chown(p, None, Some(100))?;
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o2775))
    };
    prepare(&path)?;
    for part in dir.split('/').filter(|p| !p.is_empty()) {
        path.push(part);
        if !path.exists() {
            std::fs::create_dir(&path)?;
            prepare(&path)?;
        }
    }
    Ok(())
}

pub async fn build(db: Db) -> AppManagerRef {
    match std::env::var("FERROUS_APPS").as_deref() {
        Ok("docker") | Ok("real") => match docker::DockerAppManager::connect().await {
            Ok(d) => {
                tracing::info!("apps: using real Docker backend");
                Arc::new(d)
            }
            Err(e) => {
                tracing::warn!("apps: Docker unavailable ({e}); using mock");
                Arc::new(MockAppManager::new(db))
            }
        },
        _ => {
            tracing::info!("apps: using mock backend");
            Arc::new(MockAppManager::new(db))
        }
    }
}

/// Manipulates the seeded in-memory store (default).
pub struct MockAppManager {
    pub db: Db,
    files_root: tokio::sync::RwLock<String>,
}

impl MockAppManager {
    pub fn new(db: Db) -> Self {
        Self { db, files_root: tokio::sync::RwLock::new(recipes::DEFAULT_FILES_ROOT.to_string()) }
    }
}

#[async_trait]
impl AppManager for MockAppManager {
    fn source(&self) -> &'static str {
        "mock"
    }

    async fn list(&self) -> ApiResult<Vec<InstalledApp>> {
        Ok(self.db.read().await.apps.clone())
    }

    async fn install(&self, cat: &CatalogApp, host_port: u16) -> ApiResult<InstalledApp> {
        let mut store = self.db.write().await;
        if store.apps.iter().any(|a| a.catalog_id == cat.id) {
            return Err(ApiError::Conflict(format!("{} is already installed", cat.name)));
        }
        if store.apps.iter().any(|a| a.host_port == host_port) {
            return Err(ApiError::Conflict(format!("port {host_port} is already in use")));
        }
        let mut rng = rand::thread_rng();
        let app = InstalledApp {
            id: short_id("app"),
            catalog_id: cat.id.clone(),
            name: cat.name.clone(),
            icon: cat.icon.clone(),
            category: cat.category.clone(),
            image: cat.image.clone(),
            state: AppState::Running,
            host_port,
            cpu_percent: rng.gen_range(0.5..8.0),
            mem_bytes: rng.gen_range(80u64..600) * 1_000_000,
            web_ui: Some(format!("http://{{host}}:{host_port}/")),
            hint: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        store.apps.push(app.clone());
        Ok(app)
    }

    async fn start(&self, id: &str) -> ApiResult<InstalledApp> {
        self.set_state(id, AppState::Running).await
    }

    async fn stop(&self, id: &str) -> ApiResult<InstalledApp> {
        self.set_state(id, AppState::Stopped).await
    }

    async fn files_root(&self) -> String {
        self.files_root.read().await.clone()
    }

    async fn set_files_root(&self, root: &str, _catalog: &[CatalogApp]) -> ApiResult<()> {
        *self.files_root.write().await = root.to_string();
        Ok(())
    }

    async fn uninstall(&self, id: &str) -> ApiResult<()> {
        let mut store = self.db.write().await;
        let idx = store
            .apps
            .iter()
            .position(|a| a.id == id)
            .ok_or_else(|| ApiError::NotFound(format!("app {id} not found")))?;
        store.apps.remove(idx);
        Ok(())
    }
}

impl MockAppManager {
    async fn set_state(&self, id: &str, state: AppState) -> ApiResult<InstalledApp> {
        let mut store = self.db.write().await;
        let app = store
            .apps
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or_else(|| ApiError::NotFound(format!("app {id} not found")))?;
        app.state = state;
        if state == AppState::Running {
            let mut rng = rand::thread_rng();
            app.cpu_percent = rng.gen_range(0.5..8.0);
            app.mem_bytes = rng.gen_range(80u64..600) * 1_000_000;
        } else {
            app.cpu_percent = 0.0;
            app.mem_bytes = 0;
        }
        Ok(app.clone())
    }
}
