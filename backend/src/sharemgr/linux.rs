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

use super::{do_create, do_delete, do_patch, ShareManager};
use crate::error::{ApiError, ApiResult};
use crate::models::{CreateShareReq, PatchShareReq, Share, ShareKind};
use crate::state::Db;

const DEFAULT_SMB: &str = "/etc/samba/ferrousnas-shares.conf";
const DEFAULT_NFS: &str = "/etc/exports.d/ferrousnas.exports";

pub struct LinuxShareManager {
    db: Db,
    smb_path: PathBuf,
    nfs_path: PathBuf,
    reload: bool,
}

impl LinuxShareManager {
    pub fn new(db: Db) -> Result<Self, String> {
        let smb_path = PathBuf::from(
            std::env::var("FERROUS_SMB_CONF").unwrap_or_else(|_| DEFAULT_SMB.to_string()),
        );
        let nfs_path = PathBuf::from(
            std::env::var("FERROUS_NFS_EXPORTS").unwrap_or_else(|_| DEFAULT_NFS.to_string()),
        );
        let reload = std::env::var("FERROUS_SHARES_RELOAD").as_deref() != Ok("0");

        // Fail fast (and fall back to the mock) if we can't own these files.
        probe_writable(&smb_path)?;
        probe_writable(&nfs_path)?;

        Ok(Self { db, smb_path, nfs_path, reload })
    }

    pub fn smb_path(&self) -> &Path {
        &self.smb_path
    }
    pub fn nfs_path(&self) -> &Path {
        &self.nfs_path
    }

    /// Re-render both config files from the current share set, then reload.
    async fn sync(&self) -> ApiResult<()> {
        let shares = self.db.read().await.shares.clone();

        write_atomic(&self.smb_path, &render_smb(&shares, &self.smb_path))
            .map_err(|e| ApiError::BadRequest(format!("writing {}: {e}", self.smb_path.display())))?;
        write_atomic(&self.nfs_path, &render_nfs(&shares))
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
        Ok(self.db.read().await.shares.clone())
    }

    async fn create(&self, req: CreateShareReq) -> ApiResult<Share> {
        let share = do_create(&mut *self.db.write().await, &req)?;
        self.sync().await?;
        Ok(share)
    }

    async fn patch(&self, id: &str, req: PatchShareReq) -> ApiResult<Share> {
        let share = do_patch(&mut *self.db.write().await, id, &req)?;
        self.sync().await?;
        Ok(share)
    }

    async fn delete(&self, id: &str) -> ApiResult<()> {
        do_delete(&mut *self.db.write().await, id)?;
        self.sync().await?;
        Ok(())
    }
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
        out.push_str(&format!(
            "{} *({mode},sync,no_subtree_check,root_squash)\n",
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
        if !dir.exists() {
            return Err(format!("{} does not exist", dir.display()));
        }
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
        assert!(out.contains("/mnt/tank/photos *(ro,sync,no_subtree_check,root_squash)"));
        assert!(out.contains("/mnt/tank/scratch *(rw,sync,no_subtree_check,root_squash)"));
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
