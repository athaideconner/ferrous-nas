//! Password hashing (Argon2id) and policy.
//!
//! `Argon2::default()` is Argon2id v19 with m=19 MiB, t=2, p=1 — the OWASP
//! recommended baseline. Hashes are stored in PHC string format, which embeds
//! the algorithm, parameters and salt, so parameters can be raised later
//! without invalidating existing hashes.

use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;

use crate::error::{ApiError, ApiResult};

pub const MIN_PASSWORD_LEN: usize = 8;
/// Cap the input so an enormous body can't be used to burn CPU.
pub const MAX_PASSWORD_LEN: usize = 1024;

pub fn validate_password(pw: &str) -> ApiResult<()> {
    if pw.chars().count() < MIN_PASSWORD_LEN {
        return Err(ApiError::BadRequest(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    if pw.len() > MAX_PASSWORD_LEN {
        return Err(ApiError::BadRequest("password is too long".into()));
    }
    Ok(())
}

/// Hash a password for storage. Each call uses a fresh random salt.
pub fn hash_password(pw: &str) -> ApiResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(pw.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| ApiError::BadRequest(format!("hashing password: {e}")))
}

/// Verify a password against a stored PHC hash. Never panics on malformed
/// input — a corrupt hash simply fails to verify.
pub fn verify_password(pw: &str, phc: &str) -> bool {
    if pw.len() > MAX_PASSWORD_LEN {
        return false;
    }
    match PasswordHash::new(phc) {
        Ok(parsed) => Argon2::default().verify_password(pw.as_bytes(), &parsed).is_ok(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify_round_trips() {
        let h = hash_password("correct horse battery").unwrap();
        assert!(verify_password("correct horse battery", &h));
        assert!(!verify_password("wrong horse battery", &h));
    }

    #[test]
    fn hash_is_argon2id_and_salted() {
        let a = hash_password("same-password").unwrap();
        let b = hash_password("same-password").unwrap();
        assert!(a.starts_with("$argon2id$"), "expected argon2id, got {a}");
        assert_ne!(a, b, "identical passwords must not produce identical hashes");
        // Both must still verify despite different salts.
        assert!(verify_password("same-password", &a));
        assert!(verify_password("same-password", &b));
    }

    #[test]
    fn plaintext_never_appears_in_the_hash() {
        let h = hash_password("hunter2hunter2").unwrap();
        assert!(!h.contains("hunter2"));
    }

    #[test]
    fn malformed_or_empty_hashes_fail_closed() {
        assert!(!verify_password("anything", ""));
        assert!(!verify_password("anything", "not-a-phc-string"));
        assert!(!verify_password("anything", "$argon2id$garbage"));
        // A bare plaintext "hash" must never match.
        assert!(!verify_password("hunter2", "hunter2"));
    }

    #[test]
    fn policy_rejects_short_and_oversized_passwords() {
        assert!(validate_password("short").is_err());
        assert!(validate_password("1234567").is_err());
        assert!(validate_password("12345678").is_ok());
        assert!(validate_password(&"x".repeat(MAX_PASSWORD_LEN + 1)).is_err());
    }

    #[test]
    fn oversized_password_is_rejected_at_verify_too() {
        let h = hash_password("a-real-password").unwrap();
        assert!(!verify_password(&"x".repeat(MAX_PASSWORD_LEN + 1), &h));
    }
}
