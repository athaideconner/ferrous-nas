//! Domain models for FerrousNAS.
//!
//! Everything here is a plain data type shared across the API. All values are
//! **mocked** — no model reflects real hardware. See `state.rs` for how they
//! are seeded and mutated in memory.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// System
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct SystemInfo {
    pub hostname: String,
    pub product: String,
    pub version: String,
    pub kernel: String,
    pub uptime_secs: u64,
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub load_avg: [f32; 3],
}

#[derive(Debug, Clone, Serialize)]
pub struct CpuInfo {
    pub model: String,
    pub cores: u32,
    pub threads: u32,
    pub usage_percent: f32,
    pub temp_c: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryInfo {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_used_bytes: u64,
}

/// A single point in a rolling time-series used by the dashboard charts.
#[derive(Debug, Clone, Serialize)]
pub struct StatPoint {
    /// Seconds relative to "now" (negative = in the past).
    pub t: i64,
    pub cpu_percent: f32,
    pub mem_percent: f32,
    pub net_rx_mbps: f32,
    pub net_tx_mbps: f32,
    pub disk_read_mbps: f32,
    pub disk_write_mbps: f32,
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DiskKind {
    Hdd,
    Ssd,
    Nvme,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SmartStatus {
    Passed,
    Warning,
    Failing,
}

#[derive(Debug, Clone, Serialize)]
pub struct Disk {
    pub id: String,
    pub device: String, // e.g. "sda", "nvme0n1"
    pub model: String,
    pub serial: String,
    pub size_bytes: u64,
    pub kind: DiskKind,
    pub temp_c: f32,
    pub smart: SmartStatus,
    pub power_on_hours: u32,
    /// Pool id this disk is a member of, if any.
    pub pool_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RaidLevel {
    Stripe,   // raid0
    Mirror,   // raid1
    Raidz1,   // raid5-like
    Raidz2,   // raid6-like
}

// Some variants model valid states the API can report but the seed data does
// not currently produce; the frontend handles them, so keep them.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PoolStatus {
    Online,
    Degraded,
    Offline,
    Scrubbing,
}

#[derive(Debug, Clone, Serialize)]
pub struct Pool {
    pub id: String,
    pub name: String,
    pub raid_level: RaidLevel,
    pub status: PoolStatus,
    pub size_bytes: u64,
    pub used_bytes: u64,
    pub disk_ids: Vec<String>,
    pub scrub_progress: Option<f32>, // 0..100 when scrubbing
}

#[derive(Debug, Clone, Serialize)]
pub struct Dataset {
    pub id: String,
    pub pool_id: String,
    pub name: String,
    pub path: String,
    pub used_bytes: u64,
    pub quota_bytes: Option<u64>,
    pub compression: bool,
}

// ---------------------------------------------------------------------------
// Shares
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ShareKind {
    Smb,
    Nfs,
}

#[derive(Debug, Clone, Serialize)]
pub struct Share {
    pub id: String,
    pub name: String,
    pub kind: ShareKind,
    pub dataset_id: String,
    pub path: String,
    pub enabled: bool,
    pub read_only: bool,
    pub guest_ok: bool,
    pub allowed_users: Vec<String>,
}

// ---------------------------------------------------------------------------
// Apps (Docker-style app catalog + installed apps)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AppState {
    Running,
    Stopped,
    Installing,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogApp {
    pub id: String,
    pub name: String,
    pub tagline: String,
    pub description: String,
    pub icon: String, // emoji for the mock UI
    pub category: String,
    pub image: String, // container image reference
    pub default_port: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstalledApp {
    pub id: String,
    pub catalog_id: String,
    pub name: String,
    pub icon: String,
    pub category: String,
    pub image: String,
    pub state: AppState,
    pub host_port: u16,
    pub cpu_percent: f32,
    pub mem_bytes: u64,
    pub web_ui: Option<String>,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// Users & groups
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub id: String,
    pub username: String,
    pub full_name: String,
    pub is_admin: bool,
    pub groups: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub members: Vec<String>,
}

// ---------------------------------------------------------------------------
// Network
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct NetInterface {
    pub name: String,
    pub mac: String,
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
    pub kind: String, // "ethernet" | "bridge" | "loopback"
    pub up: bool,
    pub speed_mbps: u32,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

// ---------------------------------------------------------------------------
// Alerts / notifications
// ---------------------------------------------------------------------------

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AlertLevel {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Serialize)]
pub struct Alert {
    pub id: String,
    pub level: AlertLevel,
    pub title: String,
    pub message: String,
    pub created_at: String,
    pub acknowledged: bool,
}

// ---------------------------------------------------------------------------
// Request payloads (what the frontend POSTs / PATCHes)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreatePoolReq {
    pub name: String,
    pub raid_level: RaidLevel,
    pub disk_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDatasetReq {
    pub pool_id: String,
    pub name: String,
    #[serde(default)]
    pub quota_gb: Option<u64>,
    #[serde(default)]
    pub compression: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateShareReq {
    pub name: String,
    pub kind: ShareKind,
    pub dataset_id: String,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub guest_ok: bool,
    #[serde(default)]
    pub allowed_users: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct PatchShareReq {
    pub enabled: Option<bool>,
    pub read_only: Option<bool>,
    pub guest_ok: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct InstallAppReq {
    pub catalog_id: String,
    #[serde(default)]
    pub host_port: Option<u16>,
}

#[derive(Debug, Deserialize)]
pub struct CreateUserReq {
    pub username: String,
    pub full_name: String,
    #[serde(default)]
    pub is_admin: bool,
    #[serde(default)]
    pub groups: Vec<String>,
}

/// Convenience for generating short, readable ids in mock data.
pub fn short_id(prefix: &str) -> String {
    let u = Uuid::new_v4().simple().to_string();
    format!("{prefix}-{}", &u[..8])
}
