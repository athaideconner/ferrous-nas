//! Real power backend driving `systemctl`.
//!
//! Argv only, never a shell — there's no caller-supplied input here at all
//! (reboot/shutdown take no parameters), so the attack surface `poolmgr`
//! guards against doesn't apply. The one thing worth being careful about is
//! honesty: `systemctl reboot`/`poweroff` return as soon as the request is
//! handed to systemd/logind, not when the machine actually goes down, so the
//! API response reaches the client before the machine does.

use std::process::Command;

use async_trait::async_trait;

use super::PowerManager;
use crate::error::{ApiError, ApiResult};

pub struct SystemdPowerManager;

impl SystemdPowerManager {
    /// Availability check: can we execute `systemctl` at all?
    pub fn new() -> Result<Self, String> {
        Command::new("systemctl")
            .arg("--version")
            .output()
            .map_err(|e| format!("cannot run `systemctl`: {e}"))?;
        Ok(Self)
    }
}

#[async_trait]
impl PowerManager for SystemdPowerManager {
    fn source(&self) -> &'static str {
        "systemd"
    }

    async fn reboot(&self) -> ApiResult<()> {
        tracing::warn!("power: EXECUTING `systemctl reboot`");
        run(&["reboot"])
    }

    async fn shutdown(&self) -> ApiResult<()> {
        tracing::warn!("power: EXECUTING `systemctl poweroff`");
        run(&["poweroff"])
    }
}

fn run(args: &[&str]) -> ApiResult<()> {
    let out = Command::new("systemctl")
        .args(args)
        .output()
        .map_err(|e| ApiError::BadRequest(format!("running `systemctl {}`: {e}", args.join(" "))))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(ApiError::BadRequest(format!(
            "`systemctl {}` failed: {}",
            args.join(" "),
            if err.is_empty() { "unknown error".into() } else { err }
        )));
    }
    Ok(())
}
