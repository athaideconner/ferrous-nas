//! Real Unix/Samba account provisioning via `useradd`/`userdel`/`groupadd`/
//! `groupdel`/`smbpasswd`.
//!
//! Every identifier is validated (reusing `auth::validate_username` /
//! `validate_group_name` — the same rules already applied before these names
//! ever reach `AuthStore`) before it touches argv, and commands are argv
//! vectors, never a shell string.
//!
//! `useradd`/`groupadd` succeeding *or* reporting "already exists" both count
//! as success (idempotent create); `userdel`/`groupdel` succeeding *or*
//! reporting "doesn't exist" both count as success (idempotent remove) — see
//! [`ensure`] / [`ensure_absent`]. Exit codes follow the shadow-utils
//! convention shared by useradd/userdel/groupadd/groupdel across distros
//! (`E_NAME_IN_USE=9`, `E_NOTFOUND=6`), with a stderr-text fallback for any
//! toolchain that differs.

use std::io::Write;
use std::process::{Command, Output, Stdio};

use async_trait::async_trait;

use super::UserOps;
use crate::auth::{validate_group_name, validate_username};
use crate::error::{ApiError, ApiResult};

/// shadow-utils: useradd/groupadd exit this when the name already exists.
const E_NAME_IN_USE: i32 = 9;
/// shadow-utils: userdel/groupdel exit this when there's nothing to remove.
const E_NOTFOUND: i32 = 6;

pub struct LinuxUserOps;

impl LinuxUserOps {
    /// Availability check: can we execute the core account-management tools
    /// at all? (`smbpasswd` is optional and checked per-call — see the module
    /// doc on why that failure is soft.)
    pub fn new() -> Result<Self, String> {
        for bin in ["useradd", "userdel", "groupadd", "groupdel"] {
            Command::new(bin)
                .arg("--help")
                .output()
                .map_err(|e| format!("cannot run `{bin}`: {e}"))?;
        }
        Ok(Self)
    }
}

#[async_trait]
impl UserOps for LinuxUserOps {
    fn source(&self) -> &'static str {
        "linux"
    }

    async fn provision_user(&self, username: &str, groups: &[String], password: Option<&str>) -> ApiResult<()> {
        validate_username(username)?;
        for g in groups {
            validate_group_name(g)?;
        }

        // Groups must exist before `useradd --groups` can reference them.
        for g in groups {
            ensure("groupadd", &[g])?;
        }

        let mut args: Vec<String> =
            vec!["--system".into(), "--no-create-home".into(), "--shell".into(), "/usr/sbin/nologin".into()];
        if !groups.is_empty() {
            args.push("--groups".into());
            args.push(groups.join(","));
        }
        args.push(username.to_string());
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        ensure("useradd", &argv)?;

        if let Some(pw) = password {
            if let Err(e) = set_samba_password(username, pw) {
                // Soft failure — see module doc: the OS account exists; only
                // Samba auth for this user won't work until fixed manually.
                tracing::warn!(
                    "users: created OS account '{username}' but could not set its Samba \
                     password ({e}); SMB shares will reject it until `smbpasswd {username}` is \
                     run manually"
                );
            }
        }
        Ok(())
    }

    async fn deprovision_user(&self, username: &str) -> ApiResult<()> {
        validate_username(username)?;
        // Best-effort: see module doc on why an orphaned smbpasswd entry
        // isn't the failure mode that matters here.
        let _ = run("smbpasswd", &["-x", username]);
        ensure_absent("userdel", &[username])
    }

    async fn ensure_group(&self, name: &str) -> ApiResult<()> {
        validate_group_name(name)?;
        ensure("groupadd", &[name])
    }

    async fn remove_group(&self, name: &str) -> ApiResult<()> {
        validate_group_name(name)?;
        ensure_absent("groupdel", &[name])
    }
}

fn run(cmd: &str, args: &[&str]) -> ApiResult<Output> {
    Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| ApiError::BadRequest(format!("running `{cmd} {}`: {e}", args.join(" "))))
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).trim().to_string()
}

fn command_failed(cmd: &str, args: &[&str], err: &str) -> ApiError {
    ApiError::BadRequest(format!(
        "`{cmd} {}` failed: {}",
        args.join(" "),
        if err.is_empty() { "unknown error" } else { err }
    ))
}

/// Run a create-if-missing command: success or "already exists" both count as
/// the goal being met.
fn ensure(cmd: &str, args: &[&str]) -> ApiResult<()> {
    let out = run(cmd, args)?;
    if out.status.success() {
        return Ok(());
    }
    let err = stderr_of(&out);
    if out.status.code() == Some(E_NAME_IN_USE) || err.to_ascii_lowercase().contains("already exists") {
        return Ok(());
    }
    Err(command_failed(cmd, args, &err))
}

/// Run a remove-if-present command: success or "doesn't exist" both count as
/// the goal being met. Anything else (e.g. "user is logged in", "is the
/// primary group of ...") is a real error and must not be swallowed.
fn ensure_absent(cmd: &str, args: &[&str]) -> ApiResult<()> {
    let out = run(cmd, args)?;
    if out.status.success() {
        return Ok(());
    }
    let err = stderr_of(&out);
    if out.status.code() == Some(E_NOTFOUND) || err.to_ascii_lowercase().contains("does not exist") {
        return Ok(());
    }
    Err(command_failed(cmd, args, &err))
}

/// `smbpasswd -s -a` reads the new password twice from stdin, non-interactively.
fn set_samba_password(username: &str, password: &str) -> Result<(), String> {
    let mut child = Command::new("smbpasswd")
        .args(["-s", "-a", username])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("running `smbpasswd`: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        writeln!(stdin, "{password}").map_err(|e| e.to_string())?;
        writeln!(stdin, "{password}").map_err(|e| e.to_string())?;
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(())
}
