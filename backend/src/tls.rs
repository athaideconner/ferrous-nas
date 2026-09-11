//! TLS for the daemon's own HTTPS listener.
//!
//! FerrousNAS terminates TLS itself by default — no reverse proxy required —
//! using a self-signed certificate generated on first boot and reused after
//! that. This is the same "safe by doing nothing" posture as auth: the browser
//! will show a one-time self-signed warning, but every session cookie and
//! password submission after that point travels encrypted.
//!
//! An operator who already has a real certificate (Let's Encrypt, an internal
//! CA) points `FERROUS_TLS_CERT`/`FERROUS_TLS_KEY` at it instead — see
//! `main.rs`. An operator who terminates TLS at a reverse proxy in front of
//! FerrousNAS sets `FERROUS_TLS=off` and `FERROUS_COOKIE_SECURE=1`.

use std::fs;
use std::net::{IpAddr, UdpSocket};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use rcgen::{generate_simple_self_signed, CertifiedKey};

pub struct TlsPaths {
    pub cert: PathBuf,
    pub key: PathBuf,
}

/// Return the cert/key pair to serve, generating a self-signed one into
/// `state_dir` on first run. A cert already on disk is reused as-is — this
/// never regenerates on its own, so a browser exception granted for it stays
/// valid across restarts.
pub fn ensure_self_signed(state_dir: &Path) -> Result<TlsPaths, String> {
    let cert = state_dir.join("tls-cert.pem");
    let key = state_dir.join("tls-key.pem");

    if cert.exists() && key.exists() {
        return Ok(TlsPaths { cert, key });
    }

    fs::create_dir_all(state_dir).map_err(|e| format!("creating {}: {e}", state_dir.display()))?;

    let hostname = local_hostname();
    let lan_ip = detect_primary_ip();
    let sans = build_sans(&hostname, lan_ip);

    let CertifiedKey { cert: c, signing_key } = generate_simple_self_signed(sans.clone())
        .map_err(|e| format!("generating self-signed certificate: {e}"))?;

    write_private(&cert, c.pem().as_bytes(), 0o644)?;
    write_private(&key, signing_key.serialize_pem().as_bytes(), 0o600)?;

    tracing::info!(
        "tls: generated a self-signed certificate covering [{}] at {}",
        sans.join(", "),
        cert.display()
    );

    Ok(TlsPaths { cert, key })
}

/// The SAN list for the cert: loopback names always, plus the machine's
/// hostname and best-guess LAN IP when they look usable. Kept pure so the
/// logic is testable without touching the filesystem.
fn build_sans(hostname: &str, lan_ip: Option<IpAddr>) -> Vec<String> {
    let mut sans = vec!["localhost".to_string(), "127.0.0.1".to_string(), "::1".to_string()];

    let h = hostname.trim();
    if is_valid_dns_label(h) && !sans.contains(&h.to_string()) {
        sans.push(h.to_string());
    }
    if let Some(ip) = lan_ip {
        let s = ip.to_string();
        if !sans.contains(&s) {
            sans.push(s);
        }
    }
    sans
}

/// Conservative enough for a SAN entry: rcgen will reject anything worse, but
/// checking here means a garbled hostname degrades to "cert without it"
/// rather than failing certificate generation entirely.
fn is_valid_dns_label(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

fn local_hostname() -> String {
    fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "ferrous-nas".to_string())
}

/// Best-effort outbound-interface IP, via the classic UDP "connect" trick: no
/// packet is actually sent (UDP `connect` only picks a route), so this works
/// offline too as long as a default route exists. `None` on any failure.
fn detect_primary_ip() -> Option<IpAddr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    sock.local_addr().ok().map(|a| a.ip())
}

/// Write via a temp file with permissions set before the rename, so the final
/// path never has a moment with the wrong mode.
fn write_private(path: &Path, data: &[u8], mode: u32) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data).map_err(|e| format!("writing {}: {e}", tmp.display()))?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))
        .map_err(|e| format!("chmod {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path)
        .map_err(|e| format!("renaming {} -> {}: {e}", tmp.display(), path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sans_always_include_loopback_and_localhost() {
        let sans = build_sans("", None);
        assert!(sans.contains(&"localhost".to_string()));
        assert!(sans.contains(&"127.0.0.1".to_string()));
        assert!(sans.contains(&"::1".to_string()));
    }

    #[test]
    fn sans_include_a_valid_hostname() {
        let sans = build_sans("ferrous-nas", None);
        assert!(sans.contains(&"ferrous-nas".to_string()));
    }

    #[test]
    fn sans_skip_a_hostname_that_is_not_a_valid_dns_label() {
        // A garbled hostname must degrade gracefully, not corrupt the SAN list.
        let sans = build_sans("not a hostname!", None);
        assert!(!sans.iter().any(|s| s.contains(' ') || s.contains('!')));
    }

    #[test]
    fn sans_include_the_lan_ip_when_present() {
        let ip: IpAddr = "192.168.1.50".parse().unwrap();
        let sans = build_sans("box", Some(ip));
        assert!(sans.contains(&"192.168.1.50".to_string()));
    }

    #[test]
    fn sans_have_no_duplicates_when_hostname_collides_with_a_default() {
        let sans = build_sans("localhost", None);
        assert_eq!(sans.iter().filter(|s| *s == "localhost").count(), 1);
    }

    #[test]
    fn existing_cert_and_key_are_reused_verbatim() {
        let dir = std::env::temp_dir().join("ferrous-tls-test-reuse");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("tls-cert.pem"), "existing-cert").unwrap();
        fs::write(dir.join("tls-key.pem"), "existing-key").unwrap();

        let paths = ensure_self_signed(&dir).unwrap();
        assert_eq!(fs::read_to_string(&paths.cert).unwrap(), "existing-cert");
        assert_eq!(fs::read_to_string(&paths.key).unwrap(), "existing-key");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_fresh_directory_gets_a_valid_pem_pair_with_a_private_key() {
        let dir = std::env::temp_dir().join("ferrous-tls-test-gen");
        let _ = fs::remove_dir_all(&dir);

        let paths = ensure_self_signed(&dir).unwrap();
        let cert_pem = fs::read_to_string(&paths.cert).unwrap();
        let key_pem = fs::read_to_string(&paths.key).unwrap();
        assert!(cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(key_pem.contains("PRIVATE KEY"));

        let key_mode = fs::metadata(&paths.key).unwrap().permissions().mode();
        assert_eq!(key_mode & 0o777, 0o600, "private key must not be group/world readable");

        fs::remove_dir_all(&dir).ok();
    }
}
