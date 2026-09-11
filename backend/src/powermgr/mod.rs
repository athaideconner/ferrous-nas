//! Power actions (reboot / shutdown).
//!
//! Served through the [`PowerManager`] trait:
//!
//! - [`MockPowerManager`] — the default. Acknowledges the request; the machine
//!   this daemon runs on is untouched.
//! - [`systemd::SystemdPowerManager`] — drives real `systemctl reboot` /
//!   `systemctl poweroff`, gated behind `FERROUS_POWER=systemd`.
//!
//! Unlike pools, this is **not** dry-run gated: a reboot doesn't destroy data
//! the way `zpool create` can, and every real NAS UI treats it as a single
//! confirmed click (the dashboard confirms client-side before calling this).
//! It is still opt-in like every other real-system-access subsystem, and every
//! call already requires an administrator via the route's `AdminUser` guard.

pub mod systemd;

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::ApiResult;

pub type PowerManagerRef = Arc<dyn PowerManager>;

#[async_trait]
pub trait PowerManager: Send + Sync {
    fn source(&self) -> &'static str;
    async fn reboot(&self) -> ApiResult<()>;
    async fn shutdown(&self) -> ApiResult<()>;
}

pub fn build() -> PowerManagerRef {
    match std::env::var("FERROUS_POWER").as_deref() {
        Ok("systemd") | Ok("real") => match systemd::SystemdPowerManager::new() {
            Ok(m) => {
                tracing::warn!(
                    "power: using real systemd backend — /system/reboot and /system/shutdown \
                     will really affect this machine"
                );
                Arc::new(m)
            }
            Err(e) => {
                tracing::warn!("power: systemd backend unavailable ({e}); using mock");
                Arc::new(MockPowerManager)
            }
        },
        _ => {
            tracing::info!("power: using mock backend");
            Arc::new(MockPowerManager)
        }
    }
}

/// Acknowledges without acting (default).
pub struct MockPowerManager;

#[async_trait]
impl PowerManager for MockPowerManager {
    fn source(&self) -> &'static str {
        "mock"
    }
    async fn reboot(&self) -> ApiResult<()> {
        Ok(())
    }
    async fn shutdown(&self) -> ApiResult<()> {
        Ok(())
    }
}
