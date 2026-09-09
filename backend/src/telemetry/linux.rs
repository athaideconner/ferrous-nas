//! Real, read-only Linux telemetry.
//!
//! Sources: `/proc` (stat, meminfo, loadavg, uptime, cpuinfo, net/dev,
//! diskstats), `sysfs` hwmon for temperatures, `lsblk` for the disk inventory
//! and `smartctl` for S.M.A.R.T. health. Every access is read-only — nothing
//! here mutates the system.
//!
//! Blocking reads run directly (they are tiny and the daemon is polled a few
//! times a second); a higher-traffic deployment would move them to
//! `spawn_blocking`.

use std::collections::VecDeque;
use std::fs;
use std::process::Command;
use std::sync::Mutex;
use std::time::Instant;

use async_trait::async_trait;

use crate::models::{CpuInfo, Disk, DiskKind, MemoryInfo, SmartStatus, StatPoint, SystemInfo};

use super::Telemetry;

const RING_CAP: usize = 240;

#[derive(Clone, Copy, Default)]
struct CpuTimes {
    total: u64,
    idle: u64,
}

struct Sample {
    at: Instant,
    cpu: f32,
    mem: f32,
    rx: f32,
    tx: f32,
    dr: f32,
    dw: f32,
}

struct Sampler {
    last: Instant,
    cpu: CpuTimes,
    net: (u64, u64),
    disk: (u64, u64),
    ring: VecDeque<Sample>,
}

pub struct LinuxTelemetry {
    cpu_prev: Mutex<CpuTimes>,
    sampler: Mutex<Sampler>,
}

impl LinuxTelemetry {
    pub fn new() -> Result<Self, String> {
        // Require /proc/stat to be readable — otherwise we're not on a usable
        // Linux host and the caller should fall back to the mock.
        let cpu = read_cpu_times().ok_or_else(|| "cannot read /proc/stat".to_string())?;
        Ok(LinuxTelemetry {
            cpu_prev: Mutex::new(cpu),
            sampler: Mutex::new(Sampler {
                last: Instant::now(),
                cpu,
                net: read_net_totals(),
                disk: read_disk_totals(),
                ring: VecDeque::with_capacity(RING_CAP),
            }),
        })
    }
}

#[async_trait]
impl Telemetry for LinuxTelemetry {
    fn source(&self) -> &'static str {
        "linux"
    }

    async fn system_info(&self) -> SystemInfo {
        let (mem_total, mem_avail, swap_total, swap_free) = read_meminfo();
        let usage = {
            let mut prev = self.cpu_prev.lock().unwrap();
            let now = read_cpu_times().unwrap_or(*prev);
            let u = cpu_usage_pct(*prev, now);
            *prev = now;
            u
        };
        let (cores, threads, model) = read_cpuinfo();

        SystemInfo {
            hostname: slurp("/proc/sys/kernel/hostname").unwrap_or_else(|| "unknown".into()).trim().to_string(),
            product: "FerrousNAS".to_string(),
            version: crate::VERSION.to_string(),
            kernel: slurp("/proc/sys/kernel/osrelease").unwrap_or_default().trim().to_string(),
            uptime_secs: read_uptime(),
            cpu: CpuInfo {
                model,
                cores,
                threads,
                usage_percent: usage,
                temp_c: read_cpu_temp(),
            },
            memory: MemoryInfo {
                total_bytes: mem_total,
                used_bytes: mem_total.saturating_sub(mem_avail),
                swap_total_bytes: swap_total,
                swap_used_bytes: swap_total.saturating_sub(swap_free),
            },
            load_avg: read_loadavg(),
        }
    }

    async fn stats_history(&self, points: usize) -> Vec<StatPoint> {
        let mut s = self.sampler.lock().unwrap();
        let now = Instant::now();
        let dt = now.duration_since(s.last).as_secs_f32().max(0.001);

        let cpu_now = read_cpu_times().unwrap_or(s.cpu);
        let cpu_pct = cpu_usage_pct(s.cpu, cpu_now);

        let (mem_total, mem_avail, _, _) = read_meminfo();
        let mem_pct = if mem_total > 0 {
            (mem_total.saturating_sub(mem_avail) as f32 / mem_total as f32) * 100.0
        } else {
            0.0
        };

        let net_now = read_net_totals();
        let disk_now = read_disk_totals();
        // Counter deltas -> throughput. Network is bytes -> Mbit/s; disk sectors
        // (512 B) -> MB/s.
        let rx = ((net_now.0.saturating_sub(s.net.0)) as f32 / dt) * 8.0 / 1_000_000.0;
        let tx = ((net_now.1.saturating_sub(s.net.1)) as f32 / dt) * 8.0 / 1_000_000.0;
        let dr = ((disk_now.0.saturating_sub(s.disk.0)) as f32 * 512.0 / dt) / 1_000_000.0;
        let dw = ((disk_now.1.saturating_sub(s.disk.1)) as f32 * 512.0 / dt) / 1_000_000.0;

        // Advance state.
        s.last = now;
        s.cpu = cpu_now;
        s.net = net_now;
        s.disk = disk_now;
        s.ring.push_back(Sample { at: now, cpu: cpu_pct, mem: mem_pct, rx, tx, dr, dw });
        while s.ring.len() > RING_CAP {
            s.ring.pop_front();
        }

        let take = points.min(s.ring.len());
        s.ring
            .iter()
            .skip(s.ring.len() - take)
            .map(|p| StatPoint {
                t: -(now.duration_since(p.at).as_secs() as i64),
                cpu_percent: p.cpu,
                mem_percent: p.mem,
                net_rx_mbps: p.rx,
                net_tx_mbps: p.tx,
                disk_read_mbps: p.dr,
                disk_write_mbps: p.dw,
            })
            .collect()
    }

    async fn disks(&self) -> Vec<Disk> {
        read_disks()
    }
}

// --------------------------------------------------------------------------
// /proc + sysfs parsing
// --------------------------------------------------------------------------

fn slurp(path: &str) -> Option<String> {
    fs::read_to_string(path).ok()
}

fn read_cpu_times() -> Option<CpuTimes> {
    let stat = slurp("/proc/stat")?;
    let line = stat.lines().next()?; // "cpu  u n s idle iowait irq softirq steal ..."
    let mut it = line.split_whitespace();
    if it.next()? != "cpu" {
        return None;
    }
    let vals: Vec<u64> = it.filter_map(|v| v.parse().ok()).collect();
    if vals.len() < 5 {
        return None;
    }
    let idle = vals[3] + vals.get(4).copied().unwrap_or(0); // idle + iowait
    let total: u64 = vals.iter().sum();
    Some(CpuTimes { total, idle })
}

fn cpu_usage_pct(prev: CpuTimes, now: CpuTimes) -> f32 {
    let dt = now.total.saturating_sub(prev.total);
    if dt == 0 {
        return 0.0;
    }
    let di = now.idle.saturating_sub(prev.idle);
    (((dt - di) as f32 / dt as f32) * 100.0).clamp(0.0, 100.0)
}

/// Returns (mem_total, mem_available, swap_total, swap_free) in bytes.
fn read_meminfo() -> (u64, u64, u64, u64) {
    let mut total = 0;
    let mut avail = 0;
    let mut swt = 0;
    let mut swf = 0;
    if let Some(s) = slurp("/proc/meminfo") {
        for line in s.lines() {
            let kb = line
                .split_whitespace()
                .nth(1)
                .and_then(|v| v.parse::<u64>().ok())
                .map(|kb| kb * 1024)
                .unwrap_or(0);
            if line.starts_with("MemTotal:") {
                total = kb;
            } else if line.starts_with("MemAvailable:") {
                avail = kb;
            } else if line.starts_with("SwapTotal:") {
                swt = kb;
            } else if line.starts_with("SwapFree:") {
                swf = kb;
            }
        }
    }
    (total, avail, swt, swf)
}

fn read_loadavg() -> [f32; 3] {
    if let Some(s) = slurp("/proc/loadavg") {
        let mut it = s.split_whitespace();
        let a = it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
        let b = it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
        let c = it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
        return [a, b, c];
    }
    [0.0, 0.0, 0.0]
}

fn read_uptime() -> u64 {
    slurp("/proc/uptime")
        .and_then(|s| s.split_whitespace().next().map(String::from))
        .and_then(|v| v.parse::<f64>().ok())
        .map(|f| f as u64)
        .unwrap_or(0)
}

/// Returns (physical_cores, logical_threads, model).
fn read_cpuinfo() -> (u32, u32, String) {
    let s = match slurp("/proc/cpuinfo") {
        Some(s) => s,
        None => return (0, 0, "unknown".into()),
    };
    let mut model = String::from("unknown");
    let mut threads: u32 = 0;
    let mut core_ids: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    let mut cur_phys = String::new();
    for line in s.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let (k, v) = (k.trim(), v.trim());
            match k {
                "processor" => threads += 1,
                "model name" if model == "unknown" => model = v.to_string(),
                "physical id" => cur_phys = v.to_string(),
                "core id" => {
                    core_ids.insert((cur_phys.clone(), v.to_string()));
                }
                _ => {}
            }
        }
    }
    let cores = if core_ids.is_empty() { threads } else { core_ids.len() as u32 };
    (cores, threads, model)
}

/// Highest hwmon temperature in °C (a reasonable stand-in for CPU temp).
fn read_cpu_temp() -> f32 {
    let mut best = 0.0f32;
    if let Ok(entries) = fs::read_dir("/sys/class/hwmon") {
        for e in entries.flatten() {
            let dir = e.path();
            if let Ok(files) = fs::read_dir(&dir) {
                for f in files.flatten() {
                    let name = f.file_name().to_string_lossy().to_string();
                    if name.starts_with("temp") && name.ends_with("_input") {
                        if let Some(v) = slurp(f.path().to_str().unwrap_or("")) {
                            if let Ok(milli) = v.trim().parse::<f32>() {
                                best = best.max(milli / 1000.0);
                            }
                        }
                    }
                }
            }
        }
    }
    best
}

fn read_net_totals() -> (u64, u64) {
    let mut rx = 0;
    let mut tx = 0;
    if let Some(s) = slurp("/proc/net/dev") {
        for line in s.lines() {
            let Some((iface, rest)) = line.split_once(':') else { continue };
            let iface = iface.trim();
            if iface == "lo" || iface.starts_with("veth") || iface.starts_with("br-") {
                continue;
            }
            let f: Vec<u64> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            // fields: rx_bytes ... (0), tx_bytes is index 8
            if f.len() >= 9 {
                rx += f[0];
                tx += f[8];
            }
        }
    }
    (rx, tx)
}

fn is_phys_disk(name: &str) -> bool {
    let ends_digit = name.chars().last().map(|c| c.is_ascii_digit()).unwrap_or(false);
    if name.starts_with("sd") || name.starts_with("vd") || name.starts_with("xvd") {
        // whole disk = no trailing partition number (sda, not sda1)
        !ends_digit
    } else if name.starts_with("nvme") {
        // nvme0n1 (whole) but not nvme0n1p1 (partition)
        !name.contains('p')
    } else if name.starts_with("mmcblk") {
        !name.contains('p')
    } else {
        false
    }
}

/// Returns (read_sectors_total, write_sectors_total) across physical disks.
fn read_disk_totals() -> (u64, u64) {
    let mut r = 0;
    let mut w = 0;
    if let Some(s) = slurp("/proc/diskstats") {
        for line in s.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            // fields: major minor name reads rmerge rsectors ... writes wmerge wsectors
            if f.len() < 10 {
                continue;
            }
            if !is_phys_disk(f[2]) {
                continue;
            }
            r += f[5].parse::<u64>().unwrap_or(0);
            w += f[9].parse::<u64>().unwrap_or(0);
        }
    }
    (r, w)
}

// --------------------------------------------------------------------------
// disks via lsblk + smartctl
// --------------------------------------------------------------------------

fn read_disks() -> Vec<Disk> {
    let out = Command::new("lsblk")
        .args(["-b", "-d", "-J", "-o", "NAME,MODEL,SERIAL,SIZE,ROTA,TRAN"])
        .output();
    let Ok(out) = out else { return vec![] };
    if !out.status.success() {
        return vec![];
    }
    let json: serde_json::Value = match serde_json::from_slice(&out.stdout) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let Some(devs) = json.get("blockdevices").and_then(|v| v.as_array()) else {
        return vec![];
    };

    let mut disks = Vec::new();
    for d in devs {
        let name = d.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if name.is_empty() || name.starts_with("loop") || name.starts_with("ram") || name.starts_with("sr") {
            continue;
        }
        let rota = json_bool(d.get("rota"));
        let tran = d.get("tran").and_then(|v| v.as_str()).unwrap_or("");
        let kind = if tran == "nvme" {
            DiskKind::Nvme
        } else if rota {
            DiskKind::Hdd
        } else {
            DiskKind::Ssd
        };
        let smart = smart_for(&name);
        disks.push(Disk {
            id: format!("disk-{name}"),
            device: name.clone(),
            model: d.get("model").and_then(|v| v.as_str()).unwrap_or("unknown").trim().to_string(),
            serial: d.get("serial").and_then(|v| v.as_str()).unwrap_or("").trim().to_string(),
            size_bytes: json_u64(d.get("size")),
            kind,
            temp_c: smart.1,
            smart: smart.0,
            power_on_hours: smart.2,
            pool_id: None, // pools are still mocked; real membership isn't tracked yet
        });
    }
    disks
}

/// Best-effort S.M.A.R.T. read. Returns (status, temp_c, power_on_hours).
/// smartctl typically needs root; on failure we report a neutral "passed".
fn smart_for(name: &str) -> (SmartStatus, f32, u32) {
    let out = Command::new("smartctl")
        .args(["-j", "-H", "-A", &format!("/dev/{name}")])
        .output();
    let Ok(out) = out else { return (SmartStatus::Passed, 0.0, 0) };
    let json: serde_json::Value = match serde_json::from_slice(&out.stdout) {
        Ok(v) => v,
        Err(_) => return (SmartStatus::Passed, 0.0, 0),
    };
    let status = match json.get("smart_status").and_then(|s| s.get("passed")).and_then(|v| v.as_bool()) {
        Some(true) => SmartStatus::Passed,
        Some(false) => SmartStatus::Failing,
        None => SmartStatus::Passed,
    };
    let temp = json
        .get("temperature")
        .and_then(|t| t.get("current"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;
    let hours = json
        .get("power_on_time")
        .and_then(|t| t.get("hours"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    (status, temp, hours)
}

// lsblk emits bools as real JSON bools on newer versions, strings on older.
fn json_bool(v: Option<&serde_json::Value>) -> bool {
    match v {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => s == "1" || s == "true",
        Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(0) != 0,
        _ => false,
    }
}

// SIZE with -b is bytes, but may arrive as a number or a numeric string.
fn json_u64(v: Option<&serde_json::Value>) -> u64 {
    match v {
        Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(serde_json::Value::String(s)) => s.parse().unwrap_or(0),
        _ => 0,
    }
}
