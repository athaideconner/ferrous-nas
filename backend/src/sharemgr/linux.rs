//! Real SMB / NFS share backend.
//!
//! After every mutation the full share set is re-rendered to two **managed**
//! files and the services are reloaded:
//!
//! | | default path | activation |
//! |---|---|---|
//! | SMB | `/etc/samba/ferrousnas-shares.conf` | add `include = <path>` under `[global]` in `smb.conf` |
//! | NFS | `/etc/exports.d/ferrousnas.exports` | picked up by `exportfs` automatically |
//!
//! Both are overridable (`FERROUS_SMB_CONF`, `FERROUS_NFS_EXPORTS`) which also
//! makes the renderer testable without touching `/etc`. Set
//! `FERROUS_SHARES_RELOAD=0` to render config without reloading services.
//!
//! FerrousNAS never edits `smb.conf` or `/etc/exports` in place — only the two
//! files above, which it owns entirely.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use async_trait::async_trait;
use tokio::sync::Mutex;

use super::{do_create, do_delete, do_patch, ShareManager};
use crate::error::{ApiError, ApiResult};
use crate::models::{CreateShareReq, PatchShareReq, Share, ShareKind};
use crate::poolmgr::PoolManagerRef;

const DEFAULT_SMB: &str = "/etc/samba/ferrousnas-shares.conf";
const DEFAULT_NFS: &str = "/etc/exports.d/ferrousnas.exports";

/// Group every shared folder belongs to (Debian's standard `users`, gid 100).
/// SMB access is forced to it and NFS access squashed to it, so whoever
/// connects can write wherever the share allows, whatever their own uid.
const SHARE_GROUP: &str = "users";
const SHARE_GID: u32 = 100;

pub struct LinuxShareManager {
    shares: Mutex<Vec<Share>>,
    pools: PoolManagerRef,
    smb_path: PathBuf,
    nfs_path: PathBuf,
    /// Where the share list is saved, so shares survive a restart.
    state_path: PathBuf,
    reload: bool,
}

impl LinuxShareManager {
    pub async fn new(pools: PoolManagerRef) -> Result<Self, String> {
        let smb_path = PathBuf::from(
            std::env::var("FERROUS_SMB_CONF").unwrap_or_else(|_| DEFAULT_SMB.to_string()),
        );
        let nfs_path = PathBuf::from(
            std::env::var("FERROUS_NFS_EXPORTS").unwrap_or_else(|_| DEFAULT_NFS.to_string()),
        );
        let reload = std::env::var("FERROUS_SHARES_RELOAD").as_deref() != Ok("0");
        let state_path = PathBuf::from(
            std::env::var("FERROUS_STATE_DIR").unwrap_or_else(|_| "/var/lib/ferrous-nas".into()),
        )
        .join("shares.json");

        // Fail fast (and fall back to the mock) if we can't own these files.
        probe_writable(&smb_path)?;
        probe_writable(&nfs_path)?;

        let shares = match fs::read_to_string(&state_path) {
            Ok(json) => serde_json::from_str(&json)
                .map_err(|e| format!("{} is unreadable: {e}", state_path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(format!("{}: {e}", state_path.display())),
        };
        let m = Self { shares: Mutex::new(shares), pools, smb_path, nfs_path, state_path, reload };
        // Bring the config files in line with the saved list (e.g. after a
        // crash between saving and rendering).
        m.sync(&m.shares.lock().await).map_err(|e| e.to_string())?;
        Ok(m)
    }

    pub fn smb_path(&self) -> &Path {
        &self.smb_path
    }
    pub fn nfs_path(&self) -> &Path {
        &self.nfs_path
    }

    /// Save the share list, re-render both config files from it, then reload.
    fn sync(&self, shares: &[Share]) -> ApiResult<()> {
        let json = serde_json::to_string_pretty(shares).unwrap_or_default();
        write_atomic(&self.state_path, &json)
            .map_err(|e| ApiError::BadRequest(format!("saving {}: {e}", self.state_path.display())))?;
        write_atomic(&self.smb_path, &render_smb(shares, &self.smb_path))
            .map_err(|e| ApiError::BadRequest(format!("writing {}: {e}", self.smb_path.display())))?;
        write_atomic(&self.nfs_path, &render_nfs(shares))
            .map_err(|e| ApiError::BadRequest(format!("writing {}: {e}", self.nfs_path.display())))?;

        if self.reload {
            reload_services();
        }
        Ok(())
    }
}

#[async_trait]
impl ShareManager for LinuxShareManager {
    fn source(&self) -> &'static str {
        "linux"
    }

    async fn list(&self) -> ApiResult<Vec<Share>> {
        Ok(self.shares.lock().await.clone())
    }

    async fn create(&self, req: CreateShareReq) -> ApiResult<Share> {
        let datasets = self.pools.datasets().await?;
        let mut shares = self.shares.lock().await;
        let share = do_create(&mut shares, &datasets, &req)?;
        if let Err(e) = prepare_folder(Path::new(&share.path)).and_then(|_| self.sync(&shares)) {
            shares.retain(|s| s.id != share.id);
            return Err(e);
        }
        Ok(share)
    }

    async fn patch(&self, id: &str, req: PatchShareReq) -> ApiResult<Share> {
        let mut shares = self.shares.lock().await;
        let share = do_patch(&mut shares, id, &req)?;
        self.sync(&shares)?;
        Ok(share)
    }

    async fn delete(&self, id: &str) -> ApiResult<()> {
        let mut shares = self.shares.lock().await;
        do_delete(&mut shares, id)?;
        self.sync(&shares)?;
        Ok(())
    }
}

/// Make a shared folder writable through the share: group [`SHARE_GROUP`],
/// group-writable, setgid so new files and folders inherit the group. Only
/// the folder itself — existing contents keep their permissions.
fn prepare_folder(path: &Path) -> ApiResult<()> {
    use std::os::unix::fs::PermissionsExt;
    let fail = |e: std::io::Error| ApiError::BadRequest(format!("preparing {}: {e}", path.display()));
    std::os::unix::fs::chown(path, None, Some(SHARE_GID)).map_err(fail)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o2775)).map_err(fail)
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

/// Strip anything that could break out of a config line/stanza.
fn sanitize(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() && *c != '[' && *c != ']')
        .collect::<String>()
        .trim()
        .to_string()
}

pub(crate) fn render_smb(shares: &[Share], self_path: &Path) -> String {
    let mut out = String::new();
    out.push_str("# Managed by FerrousNAS — do not edit by hand.\n");
    out.push_str("# Regenerated from the FerrousNAS share list on every change.\n#\n");
    out.push_str("# To activate, add this line under [global] in /etc/samba/smb.conf:\n");
    out.push_str(&format!("#     include = {}\n", self_path.display()));

    for s in shares.iter().filter(|s| s.kind == ShareKind::Smb && s.enabled) {
        out.push_str(&format!("\n[{}]\n", sanitize(&s.name)));
        out.push_str(&format!("   path = {}\n", sanitize(&s.path)));
        out.push_str("   browseable = yes\n");
        out.push_str(&format!("   read only = {}\n", yes_no(s.read_only)));
        out.push_str(&format!("   guest ok = {}\n", yes_no(s.guest_ok)));
        // See SHARE_GROUP: files are created in the folder's group, and
        // stay writable by everyone who can use the share.
        out.push_str(&format!("   force group = {SHARE_GROUP}\n"));
        out.push_str("   create mask = 0664\n");
        out.push_str("   directory mask = 2775\n");
        if !s.allowed_users.is_empty() {
            let users: Vec<String> = s.allowed_users.iter().map(|u| sanitize(u)).collect();
            out.push_str(&format!("   valid users = {}\n", users.join(" ")));
        }
    }
    out
}

pub(crate) fn render_nfs(shares: &[Share]) -> String {
    let mut out = String::new();
    out.push_str("# Managed by FerrousNAS — do not edit by hand.\n");
    out.push_str("# Regenerated from the FerrousNAS share list on every change.\n");

    for s in shares.iter().filter(|s| s.kind == ShareKind::Nfs && s.enabled) {
        let mode = if s.read_only { "ro" } else { "rw" };
        // NFS trusts client uids, which don't match this machine's; squash
        // everyone to nobody in SHARE_GROUP, which can write the folder.
        out.push_str(&format!(
            "{} *({mode},sync,no_subtree_check,all_squash,anonuid=65534,anongid={SHARE_GID})\n",
            sanitize(&s.path)
        ));
    }
    out
}

fn yes_no(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

// ---------------------------------------------------------------------------
// filesystem + services
// ---------------------------------------------------------------------------

fn probe_writable(path: &Path) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        // e.g. /etc/exports.d, which nfs-kernel-server doesn't create.
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map(|_| ())
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Write via a temp file + rename so a reader never sees a half-written config.
fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

/// Best-effort service reloads — a failure is logged, not fatal: the config is
/// already on disk and an admin can reload manually.
fn reload_services() {
    let smb_ok = run("systemctl", &["reload", "smbd"])
        || run("smbcontrol", &["all", "reload-config"]);
    if !smb_ok {
        tracing::warn!("shares: could not reload Samba (is smbd installed/running?)");
    }
    if !run("exportfs", &["-ra"]) {
        tracing::warn!("shares: could not run `exportfs -ra` (is nfs-kernel-server installed?)");
    }
}

fn run(cmd: &str, args: &[&str]) -> bool {
    match Command::new(cmd).args(args).output() {
        Ok(o) if o.status.success() => true,
        Ok(o) => {
            tracing::debug!(
                "shares: {cmd} {:?} failed: {}",
                args,
                String::from_utf8_lossy(&o.stderr).trim()
            );
            false
        }
        Err(e) => {
            tracing::debug!("shares: {cmd} not runnable: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ShareKind;

    fn share(name: &str, kind: ShareKind, enabled: bool, ro: bool, guest: bool, users: &[&str]) -> Share {
        Share {
            id: format!("share-{name}"),
            name: name.to_string(),
            kind,
            dataset_id: "ds-1".into(),
            path: format!("/mnt/tank/{}", name.to_lowercase()),
            enabled,
            read_only: ro,
            guest_ok: guest,
            allowed_users: users.iter().map(|u| u.to_string()).collect(),
        }
    }

    #[test]
    fn smb_renders_only_enabled_smb_shares() {
        let shares = vec![
            share("Media", ShareKind::Smb, true, false, true, &[]),
            share("Off", ShareKind::Smb, false, false, false, &[]),
            share("Nfsy", ShareKind::Nfs, true, false, false, &[]),
        ];
        let out = render_smb(&shares, Path::new("/etc/samba/ferrousnas-shares.conf"));
        assert!(out.contains("[Media]"));
        assert!(out.contains("   path = /mnt/tank/media"));
        assert!(out.contains("   read only = no"));
        assert!(out.contains("   guest ok = yes"));
        assert!(out.contains("   force group = users"));
        assert!(!out.contains("[Off]"), "disabled share must not be exported");
        assert!(!out.contains("[Nfsy]"), "NFS share must not appear in smb.conf");
    }

    #[test]
    fn smb_emits_valid_users_when_restricted() {
        let shares = vec![share("Docs", ShareKind::Smb, true, true, false, &["gorav", "alex"])];
        let out = render_smb(&shares, Path::new("/tmp/x.conf"));
        assert!(out.contains("   read only = yes"));
        assert!(out.contains("   valid users = gorav alex"));
    }

    #[test]
    fn smb_sanitizes_names_that_could_break_the_stanza() {
        let mut s = share("Ev[il]", ShareKind::Smb, true, false, false, &[]);
        s.path = "/mnt/tank/ok".into();
        let out = render_smb(&[s], Path::new("/tmp/x.conf"));
        assert!(out.contains("[Evil]"), "brackets must be stripped, got:\n{out}");
    }

    #[test]
    fn nfs_renders_rw_and_ro_exports() {
        let shares = vec![
            share("Photos", ShareKind::Nfs, true, true, false, &[]),
            share("Scratch", ShareKind::Nfs, true, false, false, &[]),
            share("Disabled", ShareKind::Nfs, false, false, false, &[]),
            share("Smbish", ShareKind::Smb, true, false, false, &[]),
        ];
        let out = render_nfs(&shares);
        assert!(out.contains("/mnt/tank/photos *(ro,sync,no_subtree_check,all_squash,anonuid=65534,anongid=100)"));
        assert!(out.contains("/mnt/tank/scratch *(rw,sync,no_subtree_check,all_squash,anonuid=65534,anongid=100)"));
        assert!(!out.contains("disabled"));
        assert!(!out.contains("smbish"), "SMB share must not appear in exports");
    }

    #[test]
    fn write_atomic_replaces_contents() {
        let dir = std::env::temp_dir().join("ferrous-share-test");
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("out.conf");
        write_atomic(&p, "first").unwrap();
        write_atomic(&p, "second").unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "second");
        fs::remove_dir_all(&dir).ok();
    }
}
