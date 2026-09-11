//! Real OS-account provisioning for dashboard users and groups.
//!
//! [`crate::auth::AuthStore`] owns the dashboard-level account (who can log in
//! to FerrousNAS); this module owns the optional, separate concern of making
//! that same identity usable at the OS level — a real Unix account, and a
//! Samba password, so a share's rendered `valid users = gorav`
//! ([`crate::sharemgr::linux`]) is actually enforceable.
//!
//! Served through the [`UserOps`] trait:
//!
//! - [`NoopUserOps`] — the default. Dashboard accounts exist only in
//!   `auth.json`; no OS account is touched.
//! - [`linux::LinuxUserOps`] — drives `useradd`/`userdel`/`groupadd`/
//!   `groupdel`/`smbpasswd`, gated behind `FERROUS_USERS=linux`.
//!
//! **Failure policy is asymmetric on purpose:**
//!
//! - Provisioning a login-capable account and failing to fully create it is a
//!   correctness problem the caller must not paper over — [`api::users`]
//!   rolls back the dashboard-level user if [`UserOps::provision_user`] fails,
//!   so there is never a "dashboard login works, no real account" ghost.
//! - Deprovisioning and failing to actually remove the OS account is a
//!   *security* problem — an account the dashboard believes is gone that can
//!   still authenticate. [`UserOps::deprovision_user`] failing therefore
//!   aborts the deletion entirely (fail closed) rather than proceeding.
//! - An empty Unix *group* left behind after a failed `groupdel` is neither:
//!   no principal gains access through a leftover empty group, so that
//!   failure is only logged, never fatal to the dashboard-level change.

pub mod linux;

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::ApiResult;

pub type UserOpsRef = Arc<dyn UserOps>;

#[async_trait]
pub trait UserOps: Send + Sync {
    fn source(&self) -> &'static str;
    async fn provision_user(&self, username: &str, groups: &[String], password: Option<&str>) -> ApiResult<()>;
    async fn deprovision_user(&self, username: &str) -> ApiResult<()>;
    async fn ensure_group(&self, name: &str) -> ApiResult<()>;
    async fn remove_group(&self, name: &str) -> ApiResult<()>;
}

pub fn build() -> UserOpsRef {
    match std::env::var("FERROUS_USERS").as_deref() {
        Ok("linux") | Ok("real") => match linux::LinuxUserOps::new() {
            Ok(m) => {
                tracing::info!(
                    "users: using real Linux account backend — dashboard users get real \
                     Unix/Samba accounts"
                );
                Arc::new(m)
            }
            Err(e) => {
                tracing::warn!("users: Linux account backend unavailable ({e}); using mock");
                Arc::new(NoopUserOps)
            }
        },
        _ => {
            tracing::info!("users: using mock backend (no real OS accounts)");
            Arc::new(NoopUserOps)
        }
    }
}

/// Touches nothing outside `auth.json` (default).
pub struct NoopUserOps;

#[async_trait]
impl UserOps for NoopUserOps {
    fn source(&self) -> &'static str {
        "mock"
    }
    async fn provision_user(&self, _username: &str, _groups: &[String], _password: Option<&str>) -> ApiResult<()> {
        Ok(())
    }
    async fn deprovision_user(&self, _username: &str) -> ApiResult<()> {
        Ok(())
    }
    async fn ensure_group(&self, _name: &str) -> ApiResult<()> {
        Ok(())
    }
    async fn remove_group(&self, _name: &str) -> ApiResult<()> {
        Ok(())
    }
}
