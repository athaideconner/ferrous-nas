//! Real pool/dataset backend driving `zpool` and `zfs`.
//!
//! Safety architecture:
//!
//! * **No shell.** Commands are built as argv vectors, never a shell string.
//! * **Every caller-supplied identifier is validated** before it can reach the
//!   CLI — names must start with a letter, so a value like `-f` can never be
//!   parsed as a flag (see [`super::safety`]).
//! * **Disks must prove they are empty.** Each candidate is inspected with
//!   `lsblk`; anything with a partition table, filesystem signature, child
//!   partition, active mount, or that backs `/` is refused.
//! * **No `-f`.** We never force past ZFS's own safety checks.
//! * **Dry-run by default.** `zpool create`, `zpool destroy` and `zfs destroy`
//!   are refused with the exact command unless `FERROUS_POOLS_DESTRUCTIVE`
//!   is set to `i-understand`.

use std::process::Command;

use async_trait::async_trait;

use super::safety::{self, DiskFacts};
use super::PoolManager;
use crate::error::{ApiError, ApiResult};
use crate::models::*;

const DESTRUCTIVE_PHRASE: &str = "i-understand";

pub struct ZfsPoolManager {
    destructive: bool,
}

impl ZfsPoolManager {
    pub fn new() -> Result<Self, String> {
        // Availability check: can we execute the zpool binary at all?
        Command::new("zpool")
            .arg("list")
            .arg("-H")
            .output()
            .map_err(|e| format!("cannot run `zpool`: {e}"))?;
        Ok(Self {
            destructive: std::env::var("FERROUS_POOLS_DESTRUCTIVE").as_deref()
                == Ok(DESTRUCTIVE_PHRASE),
        })
    }

    pub fn destructive_allowed(&self) -> bool {
        self.destructive
    }

    /// Gate a data-destroying command. In dry-run this returns `403` carrying
    /// the exact argv that would have run.
    fn guard(&self, cmd: &str, args: &[&str]) -> ApiResult<()> {
        let line = cmdline(cmd, args);
        if !self.destructive {
            tracing::info!("pools: DRY-RUN, refusing destructive command: {line}");
            return Err(ApiError::Forbidden(format!(
                "dry-run: refused to run `{line}`. Set FERROUS_POOLS_DESTRUCTIVE={DESTRUCTIVE_PHRASE} to allow destructive pool operations."
            )));
        }
        tracing::warn!("pools: EXECUTING destructive command: {line}");
        Ok(())
    }
}

#[async_trait]
impl PoolManager for ZfsPoolManager {
    fn source(&self) -> &'static str {
        "zfs"
    }

    async fn pools(&self) -> ApiResult<Vec<Pool>> {
        let out = run("zpool", &["list", "-H", "-p", "-v", "-o", "name,size,alloc,health"])?;
        Ok(parse_zpool_list(&out))
    }

    async fn create_pool(&self, req: CreatePoolReq) -> ApiResult<Pool> {
        safety::validate_pool_name(&req.name)?;
        safety::check_disk_count(req.raid_level, req.disk_ids.len())?;

        if self.pools().await?.iter().any(|p| p.name == req.name) {
            return Err(ApiError::Conflict(format!("pool '{}' already exists", req.name)));
        }

        // Resolve ids -> device names, then prove every disk is blank.
        let root = root_disk();
        let mut devices = Vec::new();
        for id in &req.disk_ids {
            let dev = device_name(id)?;
            let facts = inspect_disk(&dev, root.as_deref())?;
            safety::check_disk_safe(&facts)
                .map_err(|why| ApiError::Forbidden(format!("refusing to use {dev}: {why}")))?;
            devices.push(format!("/dev/{dev}"));
        }

        // zpool create <name> [vdev-keyword] <devices...>   (never -f)
        let mut args: Vec<&str> = vec!["create", &req.name];
        if let Some(kw) = safety::vdev_keyword(req.raid_level) {
            args.push(kw);
        }
        for d in &devices {
            args.push(d);
        }

        self.guard("zpool", &args)?;
        run("zpool", &args)?;

        self.pools()
            .await?
            .into_iter()
            .find(|p| p.name == req.name)
            .ok_or_else(|| ApiError::BadRequest("pool created but not found".into()))
    }

    async fn delete_pool(&self, id: &str) -> ApiResult<()> {
        safety::validate_pool_name(id)?;
        let args = ["destroy", id];
        self.guard("zpool", &args)?;
        run("zpool", &args)?;
        Ok(())
    }

    async fn scrub_pool(&self, id: &str) -> ApiResult<Pool> {
        // Scrubbing is a read/repair pass, not destructive — allowed in dry-run.
        safety::validate_pool_name(id)?;
        run("zpool", &["scrub", id])?;
        self.pools()
            .await?
            .into_iter()
            .find(|p| p.name == id)
            .ok_or_else(|| ApiError::NotFound(format!("pool {id} not found")))
    }

    async fn datasets(&self) -> ApiResult<Vec<Dataset>> {
        let out = run(
            "zfs",
            &["list", "-H", "-p", "-t", "filesystem", "-o", "name,used,quota,compression,mountpoint"],
        )?;
        Ok(parse_zfs_list(&out))
    }

    async fn create_dataset(&self, req: CreateDatasetReq) -> ApiResult<Dataset> {
        // Creating a dataset adds data, it doesn't destroy any — not gated.
        safety::validate_pool_name(&req.pool_id)?;
        safety::validate_dataset_name(&req.name)?;
        let full = format!("{}/{}", req.pool_id, req.name);

        let mut args: Vec<String> = vec!["create".into()];
        if req.compression {
            args.push("-o".into());
            args.push("compression=lz4".into());
        }
        if let Some(gb) = req.quota_gb {
            args.push("-o".into());
            args.push(format!("quota={}", gb * super::GB));
        }
        args.push(full.clone());

        let argv: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        tracing::info!("pools: {}", cmdline("zfs", &argv));
        run("zfs", &argv)?;

        self.datasets()
            .await?
            .into_iter()
            .find(|d| format!("{}/{}", d.pool_id, d.name) == full)
            .ok_or_else(|| ApiError::BadRequest("dataset created but not found".into()))
    }

    async fn delete_dataset(&self, id: &str) -> ApiResult<()> {
        validate_dataset_path(id)?;
        let args = ["destroy", id];
        self.guard("zfs", &args)?;
        run("zfs", &args)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// identifier handling
// ---------------------------------------------------------------------------

/// Map a disk id (`disk-sda`, or a bare `sda`) to a validated device name.
/// Deliberately strict: only letters and digits, so nothing can escape into
/// the argv as a path or a flag.
fn device_name(id: &str) -> ApiResult<String> {
    let dev = id.strip_prefix("disk-").unwrap_or(id);
    if dev.is_empty()
        || dev.len() > 32
        || !dev.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return Err(ApiError::BadRequest(format!("invalid disk identifier '{id}'")));
    }
    Ok(dev.to_string())
}

/// Validate a `pool/dataset` identifier before it reaches `zfs destroy`.
fn validate_dataset_path(id: &str) -> ApiResult<()> {
    let (pool, name) = id
        .split_once('/')
        .ok_or_else(|| ApiError::BadRequest(format!("'{id}' is not a pool/dataset path")))?;
    safety::validate_pool_name(pool)?;
    safety::validate_dataset_name(name)
}

// ---------------------------------------------------------------------------
// command execution
// ---------------------------------------------------------------------------

fn cmdline(cmd: &str, args: &[&str]) -> String {
    format!("{cmd} {}", args.join(" "))
}

fn run(cmd: &str, args: &[&str]) -> ApiResult<String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| ApiError::BadRequest(format!("running `{}`: {e}", cmdline(cmd, args))))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(ApiError::BadRequest(format!(
            "`{}` failed: {}",
            cmdline(cmd, args),
            if err.is_empty() { "unknown error".into() } else { err }
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// The disk backing `/`, so we can refuse it.
fn root_disk() -> Option<String> {
    let out = Command::new("findmnt")
        .args(["-n", "-o", "SOURCE", "/"])
        .output()
        .ok()?;
    safety::root_disk_of(String::from_utf8_lossy(&out.stdout).trim())
}

/// Gather the facts [`safety::check_disk_safe`] needs, via `lsblk`.
fn inspect_disk(dev: &str, root: Option<&str>) -> ApiResult<DiskFacts> {
    let path = format!("/dev/{dev}");
    let out = run(
        "lsblk",
        &["-b", "-J", "-o", "NAME,TYPE,FSTYPE,PTTYPE,MOUNTPOINT", &path],
    )?;
    let json: serde_json::Value = serde_json::from_str(&out)
        .map_err(|e| ApiError::BadRequest(format!("parsing lsblk output for {dev}: {e}")))?;
    let node = json
        .get("blockdevices")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .ok_or_else(|| ApiError::BadRequest(format!("no such block device: {path}")))?;

    let mut mountpoints = Vec::new();
    collect_mountpoints(node, &mut mountpoints);
    let children = node.get("children").and_then(|v| v.as_array());

    Ok(DiskFacts {
        name: dev.to_string(),
        kind: node.get("type").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        fstype: node.get("fstype").and_then(|v| v.as_str()).map(String::from),
        pttype: node.get("pttype").and_then(|v| v.as_str()).map(String::from),
        mountpoints,
        has_children: children.is_some_and(|c| !c.is_empty()),
        is_root_disk: root == Some(dev),
    })
}

fn collect_mountpoints(node: &serde_json::Value, out: &mut Vec<String>) {
    if let Some(mp) = node.get("mountpoint").and_then(|v| v.as_str()) {
        if !mp.is_empty() {
            out.push(mp.to_string());
        }
    }
    if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
        for c in children {
            collect_mountpoints(c, out);
        }
    }
}

// ---------------------------------------------------------------------------
// output parsing (pure — unit-tested below)
// ---------------------------------------------------------------------------

/// Parse `zpool list -H -p -v -o name,size,alloc,health`.
///
/// Depth 0 rows are pools; deeper rows are either a vdev keyword row
/// (`raidz1-0`, `mirror-0`) which tells us the RAID level, or a leaf device.
pub(crate) fn parse_zpool_list(out: &str) -> Vec<Pool> {
    let mut pools: Vec<Pool> = Vec::new();
    for line in out.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let depth = line.chars().take_while(|c| *c == '\t').count();
        let cols: Vec<&str> = line.trim_matches('\t').split('\t').collect();
        let name = cols.first().copied().unwrap_or("").trim();
        if name.is_empty() {
            continue;
        }

        if depth == 0 {
            pools.push(Pool {
                id: name.to_string(),
                name: name.to_string(),
                raid_level: RaidLevel::Stripe, // refined by child rows
                status: match cols.get(3).copied().unwrap_or("").trim() {
                    "ONLINE" => PoolStatus::Online,
                    "DEGRADED" => PoolStatus::Degraded,
                    _ => PoolStatus::Offline,
                },
                size_bytes: cols.get(1).and_then(|v| v.trim().parse().ok()).unwrap_or(0),
                used_bytes: cols.get(2).and_then(|v| v.trim().parse().ok()).unwrap_or(0),
                disk_ids: Vec::new(),
                scrub_progress: None,
            });
        } else if let Some(pool) = pools.last_mut() {
            let lower = name.to_ascii_lowercase();
            if lower.starts_with("raidz2") {
                pool.raid_level = RaidLevel::Raidz2;
            } else if lower.starts_with("raidz1") || lower.starts_with("raidz") {
                pool.raid_level = RaidLevel::Raidz1;
            } else if lower.starts_with("mirror") {
                pool.raid_level = RaidLevel::Mirror;
            } else if !lower.starts_with("draid") && !lower.starts_with("spare") {
                // A leaf device.
                let dev = name.rsplit('/').next().unwrap_or(name);
                pool.disk_ids.push(format!("disk-{dev}"));
            }
        }
    }
    pools
}

/// Parse `zfs list -H -p -t filesystem -o name,used,quota,compression,mountpoint`.
/// Pool root datasets (no `/`) are skipped — they aren't user datasets.
pub(crate) fn parse_zfs_list(out: &str) -> Vec<Dataset> {
    let mut datasets = Vec::new();
    for line in out.lines() {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 5 {
            continue;
        }
        let full = cols[0].trim();
        let Some((pool, name)) = full.split_once('/') else { continue };
        let quota: u64 = cols[2].trim().parse().unwrap_or(0);
        datasets.push(Dataset {
            id: full.to_string(),
            pool_id: pool.to_string(),
            name: name.to_string(),
            path: cols[4].trim().to_string(),
            used_bytes: cols[1].trim().parse().unwrap_or(0),
            quota_bytes: if quota == 0 { None } else { Some(quota) },
            compression: !matches!(cols[3].trim(), "off" | "-" | ""),
        });
    }
    datasets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_raidz_and_mirror_pools() {
        // Tabs denote nesting, as `zpool list -v -H` emits.
        let out = "tank\t8000000000000\t5120000000000\tONLINE\n\
                   \traidz1-0\t8000000000000\t5120000000000\tONLINE\n\
                   \t\t/dev/sdb\t4000000000000\t-\tONLINE\n\
                   \t\t/dev/sdc\t4000000000000\t-\tONLINE\n\
                   flash\t1000000000000\t260000000000\tDEGRADED\n\
                   \tmirror-0\t1000000000000\t-\tDEGRADED\n\
                   \t\tnvme0n1\t1000000000000\t-\tONLINE\n";
        let pools = parse_zpool_list(out);
        assert_eq!(pools.len(), 2);

        assert_eq!(pools[0].name, "tank");
        assert_eq!(pools[0].raid_level, RaidLevel::Raidz1);
        assert_eq!(pools[0].status, PoolStatus::Online);
        assert_eq!(pools[0].size_bytes, 8_000_000_000_000);
        assert_eq!(pools[0].used_bytes, 5_120_000_000_000);
        assert_eq!(pools[0].disk_ids, vec!["disk-sdb", "disk-sdc"]);

        assert_eq!(pools[1].raid_level, RaidLevel::Mirror);
        assert_eq!(pools[1].status, PoolStatus::Degraded);
        assert_eq!(pools[1].disk_ids, vec!["disk-nvme0n1"]);
    }

    #[test]
    fn parses_a_stripe_pool_with_bare_devices() {
        let out = "scratch\t500000000000\t100\tONLINE\n\t/dev/sdd\t500000000000\t-\tONLINE\n";
        let pools = parse_zpool_list(out);
        assert_eq!(pools[0].raid_level, RaidLevel::Stripe);
        assert_eq!(pools[0].disk_ids, vec!["disk-sdd"]);
    }

    #[test]
    fn parses_datasets_and_skips_pool_roots() {
        let out = "tank\t100\t0\tlz4\t/tank\n\
                   tank/media\t3800000000000\t0\tlz4\t/tank/media\n\
                   tank/docs\t210000000000\t500000000000\toff\t/tank/docs\n";
        let ds = parse_zfs_list(out);
        assert_eq!(ds.len(), 2, "pool root must be skipped");
        assert_eq!(ds[0].id, "tank/media");
        assert_eq!(ds[0].pool_id, "tank");
        assert_eq!(ds[0].name, "media");
        assert_eq!(ds[0].quota_bytes, None, "quota 0 means unlimited");
        assert!(ds[0].compression);
        assert_eq!(ds[1].quota_bytes, Some(500_000_000_000));
        assert!(!ds[1].compression);
    }

    #[test]
    fn device_ids_are_strictly_validated() {
        assert_eq!(device_name("disk-sda").unwrap(), "sda");
        assert_eq!(device_name("nvme0n1").unwrap(), "nvme0n1");
        // Anything that could escape into argv must be refused.
        for bad in ["disk-../../etc", "sda;reboot", "-f", "disk-sda /dev/sdb", ""] {
            assert!(device_name(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn dataset_paths_are_validated_before_destroy() {
        assert!(validate_dataset_path("tank/media").is_ok());
        assert!(validate_dataset_path("tank").is_err(), "must be pool/dataset");
        assert!(validate_dataset_path("-f/media").is_err());
        assert!(validate_dataset_path("tank/a/b").is_err());
    }

    #[test]
    fn collects_mountpoints_from_children() {
        let node: serde_json::Value = serde_json::from_str(
            r#"{"name":"sda","mountpoint":null,"children":[
                 {"name":"sda1","mountpoint":"/boot"},
                 {"name":"sda2","mountpoint":"/"}]}"#,
        )
        .unwrap();
        let mut mps = Vec::new();
        collect_mountpoints(&node, &mut mps);
        assert_eq!(mps, vec!["/boot", "/"]);
    }
}
