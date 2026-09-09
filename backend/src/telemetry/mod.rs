//! Telemetry subsystem.
//!
//! Read-only signals (system info, live stats, physical disks) are served
//! through the [`Telemetry`] trait. Two implementations exist:
//!
//! - [`MockTelemetry`] — the default. Reads the seeded in-memory store, exactly
//!   as the rest of the app does. No hardware is touched.
//! - [`linux::LinuxTelemetry`] — reads real signals from `/proc`, `sysfs`,
//!   `lsblk` and `smartctl`. Read-only and safe: it never writes anything.
//!
//! Selection is by env var so the mock stays the default:
//!
//! ```text
//! FERROUS_TELEMETRY=linux   # or "real" — use the host's real telemetry
//! (unset / anything else)   # mocked
//! ```
//!
//! Only the read-only subsystems are wired to real hardware here. Pools,
//! datasets, shares, apps and users remain mocked (pool operations are the
//! destructive ones and are intentionally left for a later, carefully gated
//! step — see docs/ARCHITECTURE.md).

pub mod linux;

use std::sync::Arc;

use async_trait::async_trait;

use crate::models::{Disk, StatPoint, SystemInfo};
use crate::state::Db;

pub type TelemetryRef = Arc<dyn Telemetry>;

#[async_trait]
pub trait Telemetry: Send + Sync {
    /// A human label for the active source (shown by the API).
    fn source(&self) -> &'static str;
    async fn system_info(&self) -> SystemInfo;
    async fn stats_history(&self, points: usize) -> Vec<StatPoint>;
    async fn disks(&self) -> Vec<Disk>;
}

/// Build the telemetry source chosen by the environment. Falls back to the
/// mock (and logs) if the real source can't be initialised.
pub fn build(db: Db) -> TelemetryRef {
    match std::env::var("FERROUS_TELEMETRY").as_deref() {
        Ok("linux") | Ok("real") => match linux::LinuxTelemetry::new() {
            Ok(t) => {
                tracing::info!("telemetry: using real Linux source (read-only)");
                Arc::new(t)
            }
            Err(e) => {
                tracing::warn!("telemetry: real source unavailable ({e}); using mock");
                Arc::new(MockTelemetry { db })
            }
        },
        _ => {
            tracing::info!("telemetry: using mock source");
            Arc::new(MockTelemetry { db })
        }
    }
}

/// Serves the seeded in-memory data (default).
pub struct MockTelemetry {
    pub db: Db,
}

#[async_trait]
impl Telemetry for MockTelemetry {
    fn source(&self) -> &'static str {
        "mock"
    }
    async fn system_info(&self) -> SystemInfo {
        self.db.read().await.system_info()
    }
    async fn stats_history(&self, points: usize) -> Vec<StatPoint> {
        self.db.read().await.stats_history(points)
    }
    async fn disks(&self) -> Vec<Disk> {
        self.db.read().await.disks.clone()
    }
}
