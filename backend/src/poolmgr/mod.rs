//! Pool & dataset subsystem — the destructive tier.
//!
//! Served through the [`PoolManager`] trait:
//!
//! - [`MockPoolManager`] — the default. Mutates the seeded in-memory store.
//! - [`zfs::ZfsPoolManager`] — drives real `zpool` / `zfs` commands.
//!
//! Unlike the other subsystems this one can **destroy data**, so it is gated
//! separately and defensively:
//!
//! ```text
//! FERROUS_POOLS=zfs                      # opt in to the real backend
//! FERROUS_POOLS_DESTRUCTIVE=i-understand # additionally required to actually
//!                                        # run pool create / pool destroy /
//!                                        # dataset destroy
//! ```
//!
//! Without the second variable the real backend runs in **dry-run**: it
//! performs every validation and safety check, then refuses with `403` and
//! reports the exact command it would have run. Non-destructive operations
//! (list, scrub, dataset create) execute normally.
//!
//! Note: enabling this while other subsystems are mocked means dataset ids come
//! from ZFS while shares reference mock dataset ids. For a coherent real
//! system, enable telemetry, pools and shares together.

pub mod safety;
pub mod zfs;

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::{ApiError, ApiResult};
use crate::models::*;
use crate::state::Db;

pub type PoolManagerRef = Arc<dyn PoolManager>;

const GB: u64 = 1_000_000_000;

#[async_trait]
pub trait PoolManager: Send + Sync {
    fn source(&self) -> &'static str;
    async fn pools(&self) -> ApiResult<Vec<Pool>>;
    async fn create_pool(&self, req: CreatePoolReq) -> ApiResult<Pool>;
    async fn delete_pool(&self, id: &str) -> ApiResult<()>;
    async fn scrub_pool(&self, id: &str) -> ApiResult<Pool>;
    async fn datasets(&self) -> ApiResult<Vec<Dataset>>;
    async fn create_dataset(&self, req: CreateDatasetReq) -> ApiResult<Dataset>;
    async fn delete_dataset(&self, id: &str) -> ApiResult<()>;
}

pub fn build(db: Db) -> PoolManagerRef {
    match std::env::var("FERROUS_POOLS").as_deref() {
        Ok("zfs") | Ok("real") => match zfs::ZfsPoolManager::new() {
            Ok(m) => {
                if m.destructive_allowed() {
                    tracing::warn!(
                        "pools: using real ZFS backend with DESTRUCTIVE OPERATIONS ENABLED — \
                         zpool create/destroy and zfs destroy will really run"
                    );
                } else {
                    tracing::info!(
                        "pools: using real ZFS backend in DRY-RUN mode; destructive commands \
                         will be reported, not executed (set FERROUS_POOLS_DESTRUCTIVE=i-understand to enable)"
                    );
                }
                Arc::new(m)
            }
            Err(e) => {
                tracing::warn!("pools: ZFS backend unavailable ({e}); using mock");
                Arc::new(MockPoolManager { db })
            }
        },
        _ => {
            tracing::info!("pools: using mock backend");
            Arc::new(MockPoolManager { db })
        }
    }
}

/// Mutates the seeded in-memory store only (default).
pub struct MockPoolManager {
    pub db: Db,
}

#[async_trait]
impl PoolManager for MockPoolManager {
    fn source(&self) -> &'static str {
        "mock"
    }

    async fn pools(&self) -> ApiResult<Vec<Pool>> {
        Ok(self.db.read().await.pools.clone())
    }

    async fn create_pool(&self, req: CreatePoolReq) -> ApiResult<Pool> {
        safety::validate_pool_name(&req.name)?;
        safety::check_disk_count(req.raid_level, req.disk_ids.len())?;

        let mut store = self.db.write().await;
        if store.pools.iter().any(|p| p.name == req.name) {
            return Err(ApiError::Conflict(format!("pool '{}' already exists", req.name)));
        }

        let mut raw = 0u64;
        for id in &req.disk_ids {
            let disk = store
                .disks
                .iter()
                .find(|d| &d.id == id)
                .ok_or_else(|| ApiError::BadRequest(format!("unknown disk {id}")))?;
            if disk.pool_id.is_some() {
                return Err(ApiError::Conflict(format!(
                    "disk {} is already in a pool",
                    disk.device
                )));
            }
            raw += disk.size_bytes;
        }

        let pool = Pool {
            id: short_id("pool"),
            name: req.name.clone(),
            raid_level: req.raid_level,
            status: PoolStatus::Online,
            size_bytes: safety::usable_capacity(req.raid_level, raw, req.disk_ids.len() as u64),
            used_bytes: 0,
            disk_ids: req.disk_ids.clone(),
            scrub_progress: None,
        };
        for id in &req.disk_ids {
            if let Some(d) = store.disks.iter_mut().find(|d| &d.id == id) {
                d.pool_id = Some(pool.id.clone());
            }
        }
        store.pools.push(pool.clone());
        Ok(pool)
    }

    async fn delete_pool(&self, id: &str) -> ApiResult<()> {
        let mut store = self.db.write().await;
        let idx = store
            .pools
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| ApiError::NotFound(format!("pool {id} not found")))?;
        if store.datasets.iter().any(|d| d.pool_id == id) {
            return Err(ApiError::Conflict(
                "pool still has datasets; delete them first".into(),
            ));
        }
        for d in store.disks.iter_mut() {
            if d.pool_id.as_deref() == Some(id) {
                d.pool_id = None;
            }
        }
        store.pools.remove(idx);
        Ok(())
    }

    async fn scrub_pool(&self, id: &str) -> ApiResult<Pool> {
        let mut store = self.db.write().await;
        let pool = store
            .pools
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or_else(|| ApiError::NotFound(format!("pool {id} not found")))?;
        pool.status = PoolStatus::Scrubbing;
        pool.scrub_progress = Some(0.0);
        Ok(pool.clone())
    }

    async fn datasets(&self) -> ApiResult<Vec<Dataset>> {
        Ok(self.db.read().await.datasets.clone())
    }

    async fn create_dataset(&self, req: CreateDatasetReq) -> ApiResult<Dataset> {
        safety::validate_dataset_name(&req.name)?;
        let mut store = self.db.write().await;

        let pool_name = store
            .pools
            .iter()
            .find(|p| p.id == req.pool_id)
            .map(|p| p.name.clone())
            .ok_or_else(|| ApiError::BadRequest(format!("unknown pool {}", req.pool_id)))?;

        if store
            .datasets
            .iter()
            .any(|d| d.pool_id == req.pool_id && d.name == req.name)
        {
            return Err(ApiError::Conflict(format!(
                "dataset '{}' already exists in this pool",
                req.name
            )));
        }

        let ds = Dataset {
            id: short_id("ds"),
            pool_id: req.pool_id.clone(),
            name: req.name.clone(),
            path: format!("/mnt/{pool_name}/{}", req.name),
            used_bytes: 0,
            quota_bytes: req.quota_gb.map(|g| g * GB),
            compression: req.compression,
        };
        store.datasets.push(ds.clone());
        Ok(ds)
    }

    async fn delete_dataset(&self, id: &str) -> ApiResult<()> {
        let mut store = self.db.write().await;
        if store.shares.iter().any(|s| s.dataset_id == id) {
            return Err(ApiError::Conflict(
                "dataset is used by a share; delete the share first".into(),
            ));
        }
        let idx = store
            .datasets
            .iter()
            .position(|d| d.id == id)
            .ok_or_else(|| ApiError::NotFound(format!("dataset {id} not found")))?;
        store.datasets.remove(idx);
        Ok(())
    }
}
