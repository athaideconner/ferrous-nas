//! Safety policy for pool operations.
//!
//! Everything here is a **pure function** so the rules that stand between the
//! API and `zpool create` can be unit-tested without ZFS or real disks.
//!
//! Two classes of protection:
//!
//! 1. **Name validation** — ZFS naming rules, plus a hard rejection of names
//!    starting with `-` (which would otherwise be parsed as a flag by the
//!    `zpool`/`zfs` binaries) and of reserved vdev keywords.
//! 2. **Disk safety** — [`check_disk_safe`] refuses any device that shows a
//!    sign of holding data: a partition table, a filesystem signature, child
//!    partitions, an active mount, or membership of the root disk.

use crate::error::{ApiError, ApiResult};
use crate::models::RaidLevel;

/// vdev keywords that must never be used as a pool name.
const RESERVED: &[&str] = &[
    "mirror", "raidz", "raidz1", "raidz2", "raidz3", "draid", "draid1", "draid2", "draid3",
    "spare", "log", "cache", "special", "dedup",
];

fn validate_name(kind: &str, name: &str) -> ApiResult<()> {
    if name.is_empty() {
        return Err(ApiError::BadRequest(format!("{kind} name is required")));
    }
    if name.len() > 64 {
        return Err(ApiError::BadRequest(format!("{kind} name is too long (max 64)")));
    }
    // Must start with a letter: this also rules out a leading '-' being passed
    // through to the zpool/zfs CLI as a flag.
    if !name.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return Err(ApiError::BadRequest(format!(
            "{kind} name must start with a letter"
        )));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
    {
        return Err(ApiError::BadRequest(format!(
            "{kind} name may only contain letters, digits, and _ - . :"
        )));
    }
    Ok(())
}

pub fn validate_pool_name(name: &str) -> ApiResult<()> {
    validate_name("pool", name)?;
    if RESERVED.contains(&name.to_ascii_lowercase().as_str()) {
        return Err(ApiError::BadRequest(format!(
            "'{name}' is a reserved ZFS keyword and cannot be a pool name"
        )));
    }
    Ok(())
}

pub fn validate_dataset_name(name: &str) -> ApiResult<()> {
    validate_name("dataset", name)?;
    // Datasets are created as `pool/name`; a slash would let the caller escape
    // into another pool or nest unexpectedly.
    if name.contains('/') {
        return Err(ApiError::BadRequest(
            "dataset name may not contain '/'".to_string(),
        ));
    }
    Ok(())
}

pub fn min_disks(level: RaidLevel) -> usize {
    match level {
        RaidLevel::Stripe => 1,
        RaidLevel::Mirror => 2,
        RaidLevel::Raidz1 => 3,
        RaidLevel::Raidz2 => 4,
    }
}

pub fn check_disk_count(level: RaidLevel, n: usize) -> ApiResult<()> {
    let min = min_disks(level);
    if n < min {
        return Err(ApiError::BadRequest(format!(
            "this RAID level needs at least {min} disk(s)"
        )));
    }
    Ok(())
}

/// Usable capacity after redundancy overhead (approximate, as ZFS reports).
pub fn usable_capacity(level: RaidLevel, raw: u64, n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    match level {
        RaidLevel::Stripe => raw,
        RaidLevel::Mirror => raw / n,
        RaidLevel::Raidz1 => raw * (n - 1) / n,
        RaidLevel::Raidz2 => raw.saturating_mul(n.saturating_sub(2)) / n,
    }
}

/// The `zpool` vdev keyword for a RAID level (stripe has none).
pub fn vdev_keyword(level: RaidLevel) -> Option<&'static str> {
    match level {
        RaidLevel::Stripe => None,
        RaidLevel::Mirror => Some("mirror"),
        RaidLevel::Raidz1 => Some("raidz1"),
        RaidLevel::Raidz2 => Some("raidz2"),
    }
}

/// What we learned about a block device from `lsblk` (and `findmnt` for root).
#[derive(Debug, Clone, Default)]
pub struct DiskFacts {
    pub name: String,
    /// lsblk TYPE — we only ever accept "disk".
    pub kind: String,
    /// Filesystem signature, if any (e.g. "ext4", "zfs_member").
    pub fstype: Option<String>,
    /// Partition table type, if any (e.g. "gpt", "dos").
    pub pttype: Option<String>,
    /// Mountpoints of the device or any of its children.
    pub mountpoints: Vec<String>,
    /// Whether the device has child partitions.
    pub has_children: bool,
    /// Whether this disk backs the running root filesystem.
    pub is_root_disk: bool,
}

/// Refuse anything that looks like it holds data. Conservative by design: we
/// would rather reject a usable disk than destroy a used one.
pub fn check_disk_safe(d: &DiskFacts) -> Result<(), String> {
    if d.is_root_disk {
        return Err(format!("{} backs the root filesystem", d.name));
    }
    if d.kind != "disk" {
        return Err(format!(
            "{} is a '{}', not a whole disk",
            d.name,
            if d.kind.is_empty() { "unknown" } else { &d.kind }
        ));
    }
    if !d.mountpoints.is_empty() {
        return Err(format!(
            "{} is mounted at {}",
            d.name,
            d.mountpoints.join(", ")
        ));
    }
    if d.has_children {
        return Err(format!("{} has partitions", d.name));
    }
    if let Some(pt) = &d.pttype {
        if !pt.is_empty() {
            return Err(format!("{} has a {pt} partition table", d.name));
        }
    }
    if let Some(fs) = &d.fstype {
        if !fs.is_empty() {
            return Err(format!("{} already holds a {fs} signature", d.name));
        }
    }
    Ok(())
}

/// Given `findmnt` output for `/` (e.g. "/dev/nvme0n1p2"), return the parent
/// disk name ("nvme0n1") so we can refuse it.
pub fn root_disk_of(source: &str) -> Option<String> {
    let dev = source.strip_prefix("/dev/")?;
    let dev = dev.trim();
    if dev.is_empty() {
        return None;
    }
    // nvme0n1p2 -> nvme0n1 ; mmcblk0p1 -> mmcblk0 ; sda2 -> sda
    if let Some(idx) = dev.rfind('p') {
        let (head, tail) = dev.split_at(idx);
        if (dev.starts_with("nvme") || dev.starts_with("mmcblk"))
            && tail[1..].chars().all(|c| c.is_ascii_digit())
            && !tail[1..].is_empty()
        {
            return Some(head.to_string());
        }
    }
    Some(dev.trim_end_matches(|c: char| c.is_ascii_digit()).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_names_that_would_be_read_as_flags() {
        // The critical case: a name starting with '-' must never reach the CLI.
        assert!(validate_pool_name("-f").is_err());
        assert!(validate_pool_name("--force").is_err());
        assert!(validate_dataset_name("-o").is_err());
    }

    #[test]
    fn rejects_shell_and_path_metacharacters() {
        for bad in ["tank; rm -rf /", "tank/../evil", "tank pool", "tank$(x)", "tank\n[global]"] {
            assert!(validate_pool_name(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn rejects_reserved_vdev_keywords() {
        for bad in ["mirror", "raidz1", "RAIDZ2", "spare", "log", "cache"] {
            assert!(validate_pool_name(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn accepts_reasonable_names() {
        for ok in ["tank", "Vault2", "media-pool", "a.b_c"] {
            assert!(validate_pool_name(ok).is_ok(), "should accept {ok:?}");
        }
        assert!(validate_dataset_name("projects").is_ok());
        assert!(validate_dataset_name("a/b").is_err(), "slash must be rejected");
    }

    #[test]
    fn disk_count_enforced_per_raid_level() {
        assert!(check_disk_count(RaidLevel::Mirror, 1).is_err());
        assert!(check_disk_count(RaidLevel::Mirror, 2).is_ok());
        assert!(check_disk_count(RaidLevel::Raidz1, 2).is_err());
        assert!(check_disk_count(RaidLevel::Raidz1, 3).is_ok());
        assert!(check_disk_count(RaidLevel::Raidz2, 3).is_err());
        assert!(check_disk_count(RaidLevel::Raidz2, 4).is_ok());
        assert!(check_disk_count(RaidLevel::Stripe, 1).is_ok());
    }

    fn clean_disk() -> DiskFacts {
        DiskFacts { name: "sdb".into(), kind: "disk".into(), ..Default::default() }
    }

    #[test]
    fn accepts_a_genuinely_blank_disk() {
        assert!(check_disk_safe(&clean_disk()).is_ok());
    }

    #[test]
    fn refuses_root_disk() {
        let d = DiskFacts { is_root_disk: true, ..clean_disk() };
        assert!(check_disk_safe(&d).unwrap_err().contains("root filesystem"));
    }

    #[test]
    fn refuses_mounted_partitioned_or_formatted_disks() {
        let mounted = DiskFacts { mountpoints: vec!["/home".into()], ..clean_disk() };
        assert!(check_disk_safe(&mounted).unwrap_err().contains("mounted"));

        let parted = DiskFacts { has_children: true, ..clean_disk() };
        assert!(check_disk_safe(&parted).unwrap_err().contains("partitions"));

        let pt = DiskFacts { pttype: Some("gpt".into()), ..clean_disk() };
        assert!(check_disk_safe(&pt).unwrap_err().contains("partition table"));

        let fs = DiskFacts { fstype: Some("ext4".into()), ..clean_disk() };
        assert!(check_disk_safe(&fs).unwrap_err().contains("ext4"));

        // An existing pool member must not be silently re-used.
        let zfs = DiskFacts { fstype: Some("zfs_member".into()), ..clean_disk() };
        assert!(check_disk_safe(&zfs).is_err());
    }

    #[test]
    fn refuses_partitions_and_non_disks() {
        let part = DiskFacts { kind: "part".into(), ..clean_disk() };
        assert!(check_disk_safe(&part).unwrap_err().contains("not a whole disk"));
        let lvm = DiskFacts { kind: "lvm".into(), ..clean_disk() };
        assert!(check_disk_safe(&lvm).is_err());
    }

    #[test]
    fn derives_root_disk_from_partition() {
        assert_eq!(root_disk_of("/dev/sda2").as_deref(), Some("sda"));
        assert_eq!(root_disk_of("/dev/nvme0n1p2").as_deref(), Some("nvme0n1"));
        assert_eq!(root_disk_of("/dev/mmcblk0p1").as_deref(), Some("mmcblk0"));
        assert_eq!(root_disk_of("/dev/vda").as_deref(), Some("vda"));
        assert_eq!(root_disk_of("overlay"), None);
    }

    #[test]
    fn capacity_matches_raid_level() {
        let tb = 1_000_000_000_000u64;
        assert_eq!(usable_capacity(RaidLevel::Stripe, 4 * tb, 4), 4 * tb);
        assert_eq!(usable_capacity(RaidLevel::Mirror, 4 * tb, 2), 2 * tb);
        assert_eq!(usable_capacity(RaidLevel::Raidz1, 4 * tb, 4), 3 * tb);
        assert_eq!(usable_capacity(RaidLevel::Raidz2, 4 * tb, 4), 2 * tb);
        assert_eq!(usable_capacity(RaidLevel::Stripe, 0, 0), 0);
    }
}
