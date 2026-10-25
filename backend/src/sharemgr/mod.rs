//! Share management subsystem (SMB / NFS).
//!
//! Shares are served through the [`ShareManager`] trait, with two
//! implementations chosen by env var:
//!
//! - [`MockShareManager`] — the default. Mutates the seeded in-memory store.
//! - [`linux::LinuxShareManager`] — does everything the mock does *and* renders
//!   the resulting share set to real Samba / NFS config, then reloads the
//!   services.
//!
//! ```text
//! FERROUS_SHARES=linux   # or "real" — write real SMB/NFS config
//! (unset / anything else)    # mocked
//! ```
//!
//! **The share list is the source of truth.** The real config files are a
//! projection of it, re-rendered in full after every mutation, so the on-disk
//! config can always be regenerated from the API state. The mock keeps its
//! list in the seeded store; the real backend keeps its own, saved to the
//! state dir, and resolves datasets through the active pool backend (so real
//! shares point at real ZFS datasets, never at the mock's).
//!
//! **Nothing the admin owns is edited in place.** FerrousNAS writes only its
//! own managed files (a Samba `include` fragment and an `/etc/exports.d`
//! drop-in) — it never rewrites `smb.conf` or `/etc/exports` themselves.

pub mod linux;

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::{ApiError, ApiResult};
use crate::models::*;
use crate::poolmgr::PoolManagerRef;
use crate::state::Db;

pub type ShareManagerRef = Arc<dyn ShareManager>;

#[async_trait]
pub trait ShareManager: Send + Sync {
    fn source(&self) -> &'static str;
    async fn list(&self) -> ApiResult<Vec<Share>>;
    async fn create(&self, req: CreateShareReq) -> ApiResult<Share>;
    async fn patch(&self, id: &str, req: PatchShareReq) -> ApiResult<Share>;
    async fn delete(&self, id: &str) -> ApiResult<()>;
}

pub async fn build(db: Db, pools: PoolManagerRef) -> ShareManagerRef {
    match std::env::var("FERROUS_SHARES").as_deref() {
        Ok("linux") | Ok("real") => match linux::LinuxShareManager::new(pools).await {
            Ok(m) => {
                tracing::info!(
                    "shares: using real SMB/NFS backend (smb={}, nfs={})",
                    m.smb_path().display(),
                    m.nfs_path().display()
                );
                Arc::new(m)
            }
            Err(e) => {
                tracing::warn!("shares: real backend unavailable ({e}); using mock");
                Arc::new(MockShareManager { db })
            }
        },
        _ => {
            tracing::info!("shares: using mock backend");
            Arc::new(MockShareManager { db })
        }
    }
}

// ---------------------------------------------------------------------------
// Share-list mutations — shared by both backends so behaviour can't drift.
// ---------------------------------------------------------------------------

pub(crate) fn do_create(shares: &mut Vec<Share>, datasets: &[Dataset], req: &CreateShareReq) -> ApiResult<Share> {
    if req.name.trim().is_empty() {
        return Err(ApiError::BadRequest("share name is required".into()));
    }
    let path = datasets
        .iter()
        .find(|d| d.id == req.dataset_id)
        .map(|d| d.path.clone())
        .ok_or_else(|| ApiError::BadRequest(format!("unknown dataset {}", req.dataset_id)))?;

    if shares.iter().any(|s| s.name == req.name) {
        return Err(ApiError::Conflict(format!("share '{}' already exists", req.name)));
    }

    let share = Share {
        id: short_id("share"),
        name: req.name.clone(),
        kind: req.kind,
        dataset_id: req.dataset_id.clone(),
        path,
        enabled: true,
        read_only: req.read_only,
        guest_ok: req.guest_ok,
        allowed_users: req.allowed_users.clone(),
    };
    shares.push(share.clone());
    Ok(share)
}

pub(crate) fn do_patch(shares: &mut [Share], id: &str, req: &PatchShareReq) -> ApiResult<Share> {
    let share = shares
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or_else(|| ApiError::NotFound(format!("share {id} not found")))?;
    if let Some(v) = req.enabled {
        share.enabled = v;
    }
    if let Some(v) = req.read_only {
        share.read_only = v;
    }
    if let Some(v) = req.guest_ok {
        share.guest_ok = v;
    }
    Ok(share.clone())
}

pub(crate) fn do_delete(shares: &mut Vec<Share>, id: &str) -> ApiResult<()> {
    let idx = shares
        .iter()
        .position(|s| s.id == id)
        .ok_or_else(|| ApiError::NotFound(format!("share {id} not found")))?;
    shares.remove(idx);
    Ok(())
}

// ---------------------------------------------------------------------------

/// Mutates the seeded in-memory store only (default).
pub struct MockShareManager {
    pub db: Db,
}

#[async_trait]
impl ShareManager for MockShareManager {
    fn source(&self) -> &'static str {
        "mock"
    }
    async fn list(&self) -> ApiResult<Vec<Share>> {
        Ok(self.db.read().await.shares.clone())
    }
    async fn create(&self, req: CreateShareReq) -> ApiResult<Share> {
        let store = &mut *self.db.write().await;
        do_create(&mut store.shares, &store.datasets, &req)
    }
    async fn patch(&self, id: &str, req: PatchShareReq) -> ApiResult<Share> {
        do_patch(&mut self.db.write().await.shares, id, &req)
    }
    async fn delete(&self, id: &str) -> ApiResult<()> {
        do_delete(&mut self.db.write().await.shares, id)
    }
}
