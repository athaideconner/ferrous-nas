//! Authentication: users, password hashes, sessions, and groups.
//!
//! **Secure by default.** Auth is *on* unless explicitly disabled with
//! `FERROUS_AUTH=off`, and when it is disabled the daemon refuses to bind
//! anything but loopback (see `main.rs`) — so an unauthenticated FerrousNAS
//! can never be exposed to a network.
//!
//! Design:
//!
//! * **Sessions, not JWTs.** An opaque 256-bit random token in an `HttpOnly`,
//!   `SameSite=Strict` cookie: unreadable by JavaScript and revocable
//!   server-side. Sessions live in memory only, so a restart logs everyone out
//!   — correct behaviour for an appliance.
//! * **Users and groups persist, sessions don't.** Written to
//!   `<state dir>/auth.json` with mode `0600`.
//! * **No default password.** With no admin configured the API reports
//!   `setup_required` and only the setup endpoint works.
//! * **Group membership is derived, not stored twice.** A group record is just
//!   `{id, name}`; membership is computed on read by scanning users for that
//!   name in their `groups` list, so the two can never drift apart.
//!
//! This module owns dashboard-level accounts only. Provisioning the matching
//! real Unix/Samba account is a separate concern — see [`crate::usermgr`].

pub mod middleware;
pub mod password;

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::error::{ApiError, ApiResult};
use crate::models::{short_id, Group, User};

pub type AuthRef = std::sync::Arc<AuthStore>;

pub const COOKIE_NAME: &str = "ferrous_session";
/// A session dies after this long without use.
const SESSION_IDLE: Duration = Duration::from_secs(12 * 3600);
/// ...and never lives longer than this regardless of activity.
const SESSION_MAX: Duration = Duration::from_secs(30 * 24 * 3600);
/// Failed logins allowed before throttling kicks in.
const FREE_ATTEMPTS: u32 = 5;
const MAX_BACKOFF_SECS: u64 = 900;

/// A user plus their credential. **Internal only** — never serialize this to
/// an API response; convert with [`AuthUser::to_public`] instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUser {
    pub id: String,
    pub username: String,
    pub full_name: String,
    pub is_admin: bool,
    #[serde(default)]
    pub groups: Vec<String>,
    pub created_at: String,
    #[serde(default)]
    pub password_hash: Option<String>,
}

impl AuthUser {
    pub fn to_public(&self) -> User {
        User {
            id: self.id.clone(),
            username: self.username.clone(),
            full_name: self.full_name.clone(),
            is_admin: self.is_admin,
            groups: self.groups.clone(),
            created_at: self.created_at.clone(),
        }
    }
}

/// A group's persisted identity. Membership is never stored here — see the
/// module doc — so this is deliberately just a name behind an id.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GroupRecord {
    id: String,
    name: String,
}

/// The two groups a fresh install (or a pre-groups `auth.json` being loaded
/// for the first time) starts with. Matches the convention the dashboard's
/// user-creation form already assumes.
fn default_groups() -> Vec<GroupRecord> {
    vec![
        GroupRecord { id: short_id("grp"), name: "admins".to_string() },
        GroupRecord { id: short_id("grp"), name: "family".to_string() },
    ]
}

#[derive(Serialize, Deserialize, Default)]
struct Persisted {
    users: Vec<AuthUser>,
    /// `None` distinguishes "this file predates groups" (seed the defaults
    /// once) from `Some(vec![])` ("every group was deliberately deleted" —
    /// must NOT be silently reseeded on the next restart).
    #[serde(default)]
    groups: Option<Vec<GroupRecord>>,
}

struct Session {
    user_id: String,
    created: Instant,
    last_seen: Instant,
}

#[derive(Default, Clone, Copy)]
struct Throttle {
    fails: u32,
    last: Option<Instant>,
}

#[derive(Default)]
struct Inner {
    users: Vec<AuthUser>,
    groups: Vec<GroupRecord>,
    sessions: HashMap<String, Session>,
    throttle: HashMap<String, Throttle>,
}

pub struct AuthStore {
    /// When false, the middleware admits every request as a synthetic admin.
    pub enabled: bool,
    path: Option<PathBuf>,
    inner: RwLock<Inner>,
}

impl AuthStore {
    /// Build the store. When enabled, users are loaded from (and saved to)
    /// `<state dir>/auth.json`; failure to access that path is fatal rather
    /// than silently falling back to an open system.
    pub fn load(enabled: bool, seed: Vec<AuthUser>) -> Result<Self, String> {
        if !enabled {
            return Self::load_at(false, None, seed);
        }
        let dir = PathBuf::from(
            std::env::var("FERROUS_STATE_DIR").unwrap_or_else(|_| "/var/lib/ferrous-nas".into()),
        );
        Self::load_at(true, Some(dir.join("auth.json")), seed)
    }

    /// Core constructor with an explicit store path — keeps tests off the real
    /// state directory (and off a shared env var).
    pub(crate) fn load_at(
        enabled: bool,
        path: Option<PathBuf>,
        seed: Vec<AuthUser>,
    ) -> Result<Self, String> {
        if !enabled {
            // Ephemeral: seeded from the mock store, nothing written to disk.
            // Groups are always the two defaults here — there is nowhere for
            // a deliberate deletion to persist across a restart anyway.
            return Ok(Self {
                enabled: false,
                path: None,
                inner: RwLock::new(Inner { users: seed, groups: default_groups(), ..Default::default() }),
            });
        }

        let path = path.ok_or_else(|| "auth store path is required".to_string())?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
        }

        let (users, groups) = if path.exists() {
            let raw = fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;
            let p: Persisted =
                serde_json::from_str(&raw).map_err(|e| format!("parsing {}: {e}", path.display()))?;
            (p.users, p.groups.unwrap_or_else(default_groups))
        } else {
            (Vec::new(), default_groups()) // no admin yet -> setup required
        };

        Ok(Self {
            enabled: true,
            path: Some(path),
            inner: RwLock::new(Inner { users, groups, ..Default::default() }),
        })
    }

    /// True when no usable admin exists, so only the setup endpoint should work.
    pub async fn setup_required(&self) -> bool {
        if !self.enabled {
            return false;
        }
        !self
            .inner
            .read()
            .await
            .users
            .iter()
            .any(|u| u.is_admin && u.password_hash.is_some())
    }

    // -- users ------------------------------------------------------------

    pub async fn list_users(&self) -> Vec<User> {
        self.inner.read().await.users.iter().map(|u| u.to_public()).collect()
    }

    pub async fn get_user(&self, id: &str) -> Option<User> {
        self.inner.read().await.users.iter().find(|u| u.id == id).map(|u| u.to_public())
    }

    pub async fn create_user(
        &self,
        username: &str,
        full_name: &str,
        is_admin: bool,
        groups: Vec<String>,
        pw: Option<&str>,
    ) -> ApiResult<User> {
        let username = username.trim();
        validate_username(username)?;

        let hash = match pw {
            Some(pw) => {
                password::validate_password(pw)?;
                Some(password::hash_password(pw)?)
            }
            // Only meaningful when auth is disabled; a user with no hash can
            // never log in (see `login`).
            None if !self.enabled => None,
            None => return Err(ApiError::BadRequest("a password is required".into())),
        };

        let mut inner = self.inner.write().await;
        if inner.users.iter().any(|u| u.username.eq_ignore_ascii_case(username)) {
            return Err(ApiError::Conflict(format!("user '{username}' already exists")));
        }
        for g in &groups {
            if !inner.groups.iter().any(|r| &r.name == g) {
                return Err(ApiError::BadRequest(format!("unknown group '{g}'")));
            }
        }
        let user = AuthUser {
            id: short_id("user"),
            username: username.to_string(),
            full_name: full_name.to_string(),
            is_admin,
            groups,
            created_at: chrono::Utc::now().to_rfc3339(),
            password_hash: hash,
        };
        inner.users.push(user.clone());
        self.persist(&inner)?;
        Ok(user.to_public())
    }

    pub async fn delete_user(&self, id: &str) -> ApiResult<()> {
        let mut inner = self.inner.write().await;
        let idx = inner
            .users
            .iter()
            .position(|u| u.id == id)
            .ok_or_else(|| ApiError::NotFound(format!("user {id} not found")))?;

        // Never let the last administrator be removed — that would lock
        // everyone out of the appliance permanently.
        if inner.users[idx].is_admin {
            let admins = inner.users.iter().filter(|u| u.is_admin).count();
            if admins <= 1 {
                return Err(ApiError::Conflict(
                    "cannot delete the last administrator".into(),
                ));
            }
        }

        inner.users.remove(idx);
        // Revoke any sessions the deleted user still holds.
        inner.sessions.retain(|_, s| s.user_id != id);
        self.persist(&inner)?;
        Ok(())
    }

    // -- groups -------------------------------------------------------------

    /// Groups with membership computed live from the current users, so it can
    /// never drift from what `groups: [...]` on each user actually says.
    pub async fn list_groups(&self) -> Vec<Group> {
        let inner = self.inner.read().await;
        inner
            .groups
            .iter()
            .map(|g| Group {
                id: g.id.clone(),
                name: g.name.clone(),
                members: inner
                    .users
                    .iter()
                    .filter(|u| u.groups.contains(&g.name))
                    .map(|u| u.username.clone())
                    .collect(),
            })
            .collect()
    }

    pub async fn get_group(&self, id: &str) -> Option<Group> {
        self.list_groups().await.into_iter().find(|g| g.id == id)
    }

    pub async fn create_group(&self, name: &str) -> ApiResult<Group> {
        let name = name.trim();
        validate_group_name(name)?;

        let mut inner = self.inner.write().await;
        if inner.groups.iter().any(|g| g.name.eq_ignore_ascii_case(name)) {
            return Err(ApiError::Conflict(format!("group '{name}' already exists")));
        }
        let record = GroupRecord { id: short_id("grp"), name: name.to_string() };
        inner.groups.push(record.clone());
        self.persist(&inner)?;
        Ok(Group { id: record.id, name: record.name, members: vec![] })
    }

    pub async fn delete_group(&self, id: &str) -> ApiResult<()> {
        let mut inner = self.inner.write().await;
        let idx = inner
            .groups
            .iter()
            .position(|g| g.id == id)
            .ok_or_else(|| ApiError::NotFound(format!("group {id} not found")))?;

        let name = inner.groups[idx].name.clone();
        if inner.users.iter().any(|u| u.groups.contains(&name)) {
            return Err(ApiError::Conflict(
                "group still has members; remove them from the group first".into(),
            ));
        }

        inner.groups.remove(idx);
        self.persist(&inner)?;
        Ok(())
    }

    // -- authentication ----------------------------------------------------

    /// Create the first administrator. Only permitted while setup is required,
    /// so this can't be used to add an admin to a configured system.
    pub async fn setup_first_admin(
        &self,
        username: &str,
        full_name: &str,
        pw: &str,
    ) -> ApiResult<(User, String)> {
        if !self.setup_required().await {
            return Err(ApiError::Conflict("setup has already been completed".into()));
        }
        let user = self
            .create_user(username, full_name, true, vec!["admins".into()], Some(pw))
            .await?;
        let token = self.new_session(&user.id).await;
        Ok((user, token))
    }

    pub async fn login(&self, username: &str, pw: &str) -> ApiResult<(User, String)> {
        let key = username.trim().to_ascii_lowercase();

        // Throttle before doing any work.
        if let Some(wait) = self.throttle_wait(&key).await {
            return Err(ApiError::TooManyRequests(format!(
                "too many failed attempts; try again in {}s",
                wait.as_secs().max(1)
            )));
        }

        let found = {
            let inner = self.inner.read().await;
            inner
                .users
                .iter()
                .find(|u| u.username.eq_ignore_ascii_case(&key))
                .cloned()
        };

        let ok = match &found {
            Some(u) => match &u.password_hash {
                Some(h) => password::verify_password(pw, h),
                None => false,
            },
            None => {
                // Burn comparable CPU so a missing user isn't detectable by
                // response time (username enumeration).
                let _ = password::hash_password(pw);
                false
            }
        };

        if !ok {
            self.record_failure(&key).await;
            // Deliberately identical for unknown user and wrong password.
            return Err(ApiError::Unauthorized("invalid username or password".into()));
        }

        let user = found.expect("verified above");
        self.clear_failures(&key).await;
        let token = self.new_session(&user.id).await;
        Ok((user.to_public(), token))
    }

    pub async fn logout(&self, token: &str) {
        self.inner.write().await.sessions.remove(token);
    }

    /// Resolve a session token, enforcing idle and absolute lifetimes.
    pub async fn user_for_token(&self, token: &str) -> Option<AuthUser> {
        let mut inner = self.inner.write().await;

        let Some(session) = inner.sessions.get(token) else { return None };
        let now = Instant::now();
        if now.duration_since(session.last_seen) > SESSION_IDLE
            || now.duration_since(session.created) > SESSION_MAX
        {
            inner.sessions.remove(token);
            return None;
        }
        let user_id = session.user_id.clone();
        let user = inner.users.iter().find(|u| u.id == user_id).cloned();
        match user {
            Some(u) => {
                if let Some(s) = inner.sessions.get_mut(token) {
                    s.last_seen = now;
                }
                Some(u)
            }
            // User was removed out from under the session.
            None => {
                inner.sessions.remove(token);
                None
            }
        }
    }

    async fn new_session(&self, user_id: &str) -> String {
        let token = random_token();
        let now = Instant::now();
        self.inner.write().await.sessions.insert(
            token.clone(),
            Session { user_id: user_id.to_string(), created: now, last_seen: now },
        );
        token
    }

    // -- login throttling --------------------------------------------------

    async fn throttle_wait(&self, key: &str) -> Option<Duration> {
        let inner = self.inner.read().await;
        let t = inner.throttle.get(key).copied().unwrap_or_default();
        backoff_remaining(t.fails, t.last, Instant::now())
    }

    async fn record_failure(&self, key: &str) {
        let mut inner = self.inner.write().await;
        let t = inner.throttle.entry(key.to_string()).or_default();
        t.fails = t.fails.saturating_add(1);
        t.last = Some(Instant::now());
    }

    async fn clear_failures(&self, key: &str) {
        self.inner.write().await.throttle.remove(key);
    }

    // -- persistence -------------------------------------------------------

    fn persist(&self, inner: &Inner) -> ApiResult<()> {
        let Some(path) = &self.path else { return Ok(()) };
        let data = Persisted { users: inner.users.clone(), groups: Some(inner.groups.clone()) };
        write_private_json(path, &data)
            .map_err(|e| ApiError::BadRequest(format!("saving {}: {e}", path.display())))
    }
}

// ---------------------------------------------------------------------------
// helpers (pure where possible, so they can be tested)
// ---------------------------------------------------------------------------

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn validate_username(name: &str) -> ApiResult<()> {
    validate_identifier("username", name)
}

pub fn validate_group_name(name: &str) -> ApiResult<()> {
    validate_identifier("group name", name)
}

/// Shared rule set for usernames and group names: both end up as arguments to
/// real Unix commands (`useradd`, `groupadd` — see [`crate::usermgr`]) when a
/// real backend is enabled, so a name starting with `-` must never validate,
/// or it could be read as a flag by those commands.
fn validate_identifier(kind: &str, name: &str) -> ApiResult<()> {
    if name.is_empty() || name.len() > 32 {
        return Err(ApiError::BadRequest(format!("{kind} must be 1-32 characters")));
    }
    if !name.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return Err(ApiError::BadRequest(format!("{kind} must start with a letter")));
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')) {
        return Err(ApiError::BadRequest(format!(
            "{kind} may only contain letters, digits, and _ - ."
        )));
    }
    Ok(())
}

/// How long a caller must still wait, given their failure count.
/// The first [`FREE_ATTEMPTS`] are free; after that the delay doubles.
fn backoff_remaining(fails: u32, last: Option<Instant>, now: Instant) -> Option<Duration> {
    // `FREE_ATTEMPTS` attempts are allowed outright; the next one is throttled.
    if fails < FREE_ATTEMPTS {
        return None;
    }
    let last = last?;
    let exp = (fails - FREE_ATTEMPTS).min(16);
    let delay = Duration::from_secs((1u64 << exp).min(MAX_BACKOFF_SECS));
    let elapsed = now.duration_since(last);
    (elapsed < delay).then(|| delay - elapsed)
}

/// Write JSON that only the owner can read, without ever exposing a
/// world-readable window: permissions are set on the temp file before rename.
fn write_private_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    let json = serde_json::to_string_pretty(value)?;
    fs::write(&tmp, json)?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    fs::rename(&tmp, path)
}

/// Pull one cookie's value out of a `Cookie:` header.
pub fn cookie_value(header: &str, name: &str) -> Option<String> {
    header.split(';').find_map(|pair| {
        let (k, v) = pair.trim().split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

pub fn token_from_headers(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    cookie_value(raw, COOKIE_NAME)
}

fn secure_flag() -> &'static str {
    // Only mark Secure when actually served over HTTPS, otherwise the cookie
    // would be dropped on a plain-http LAN appliance and nobody could log in.
    if std::env::var("FERROUS_COOKIE_SECURE").as_deref() == Ok("1") {
        "; Secure"
    } else {
        ""
    }
}

pub fn session_cookie(token: &str) -> String {
    format!(
        "{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{}",
        SESSION_IDLE.as_secs(),
        secure_flag()
    )
}

pub fn clear_cookie() -> String {
    format!(
        "{COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{}",
        secure_flag()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each test gets its own directory, so nothing touches the real state dir
    /// and tests stay independent when run in parallel.
    fn temp_path(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ferrous-auth-test-{tag}"));
        let _ = fs::remove_dir_all(&dir);
        dir.join("auth.json")
    }

    fn temp_store(tag: &str) -> AuthStore {
        AuthStore::load_at(true, Some(temp_path(tag)), vec![]).unwrap()
    }

    #[test]
    fn cookie_parsing_finds_the_right_value() {
        let h = "theme=dark; ferrous_session=abc123; other=x";
        assert_eq!(cookie_value(h, COOKIE_NAME).as_deref(), Some("abc123"));
        assert_eq!(cookie_value("ferrous_session=solo", COOKIE_NAME).as_deref(), Some("solo"));
        assert_eq!(cookie_value("theme=dark", COOKIE_NAME), None);
        assert_eq!(cookie_value("", COOKIE_NAME), None);
        // A cookie whose name merely contains ours must not match.
        assert_eq!(cookie_value("not_ferrous_session=x", COOKIE_NAME), None);
    }

    #[test]
    fn session_cookie_has_the_hardening_attributes() {
        let c = session_cookie("tok");
        assert!(c.contains("HttpOnly"), "must be unreadable from JS: {c}");
        assert!(c.contains("SameSite=Strict"), "CSRF protection missing: {c}");
        assert!(c.contains("Path=/"));
        assert!(clear_cookie().contains("Max-Age=0"));
    }

    #[test]
    fn tokens_are_long_and_unique() {
        let a = random_token();
        let b = random_token();
        assert_eq!(a.len(), 64, "expect 32 bytes hex-encoded");
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn backoff_is_free_then_exponential() {
        let now = Instant::now();
        // Within the free allowance there is no wait at all.
        assert!(backoff_remaining(0, None, now).is_none());
        assert!(
            backoff_remaining(FREE_ATTEMPTS - 1, Some(now), now).is_none(),
            "attempts within the allowance must not be delayed"
        );
        // Once the allowance is used up, a wait appears and grows.
        let first = backoff_remaining(FREE_ATTEMPTS, Some(now), now).unwrap();
        let second = backoff_remaining(FREE_ATTEMPTS + 1, Some(now), now).unwrap();
        assert!(second > first, "backoff must increase: {first:?} then {second:?}");
        // And it is capped.
        let huge = backoff_remaining(99, Some(now), now).unwrap();
        assert!(huge <= Duration::from_secs(MAX_BACKOFF_SECS));
        // Once enough time passes, the wait clears.
        let later = now + Duration::from_secs(MAX_BACKOFF_SECS + 1);
        assert!(backoff_remaining(99, Some(now), later).is_none());
    }

    #[test]
    fn usernames_are_validated() {
        assert!(validate_username("gorav").is_ok());
        assert!(validate_username("a.b-c_1").is_ok());
        for bad in ["", "1abc", "-root", "has space", "a/b", &"x".repeat(33)] {
            assert!(validate_username(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[tokio::test]
    async fn fresh_store_requires_setup_and_refuses_login() {
        let s = temp_store("fresh");
        assert!(s.setup_required().await);
        assert!(s.login("anyone", "whatever12").await.is_err());
    }

    #[tokio::test]
    async fn setup_creates_an_admin_and_cannot_run_twice() {
        let s = temp_store("setup");
        let (user, token) = s.setup_first_admin("gorav", "Gorav", "a-good-password").await.unwrap();
        assert!(user.is_admin);
        assert!(!s.setup_required().await);
        assert_eq!(s.user_for_token(&token).await.unwrap().username, "gorav");
        // A second setup must be refused, or anyone could add themselves.
        assert!(s.setup_first_admin("evil", "Evil", "another-password").await.is_err());
    }

    #[tokio::test]
    async fn login_succeeds_then_logout_revokes_the_session() {
        let s = temp_store("login");
        s.setup_first_admin("gorav", "Gorav", "a-good-password").await.unwrap();

        assert!(s.login("gorav", "wrong-password").await.is_err());
        let (_, token) = s.login("gorav", "a-good-password").await.unwrap();
        assert!(s.user_for_token(&token).await.is_some());

        s.logout(&token).await;
        assert!(s.user_for_token(&token).await.is_none(), "token must not survive logout");
    }

    #[tokio::test]
    async fn login_is_case_insensitive_on_username_only() {
        let s = temp_store("case");
        s.setup_first_admin("Gorav", "G", "a-good-password").await.unwrap();
        assert!(s.login("gorav", "a-good-password").await.is_ok());
        assert!(s.login("GORAV", "a-good-password").await.is_ok());
        assert!(s.login("gorav", "A-GOOD-PASSWORD").await.is_err(), "password is case sensitive");
    }

    #[tokio::test]
    async fn repeated_failures_are_throttled() {
        let s = temp_store("throttle");
        s.setup_first_admin("gorav", "G", "a-good-password").await.unwrap();
        for _ in 0..FREE_ATTEMPTS {
            assert!(matches!(
                s.login("gorav", "bad-password").await,
                Err(ApiError::Unauthorized(_))
            ));
        }
        // The next attempt is refused as rate-limited, even with the right password.
        assert!(matches!(
            s.login("gorav", "a-good-password").await,
            Err(ApiError::TooManyRequests(_))
        ));
    }

    #[tokio::test]
    async fn deleting_a_user_revokes_their_sessions_and_protects_last_admin() {
        let s = temp_store("delete");
        let (admin, admin_tok) = s.setup_first_admin("gorav", "G", "a-good-password").await.unwrap();

        // Last admin is protected.
        assert!(s.delete_user(&admin.id).await.is_err());

        let bob = s
            .create_user("bob", "Bob", false, vec![], Some("bobs-password"))
            .await
            .unwrap();
        let (_, bob_tok) = s.login("bob", "bobs-password").await.unwrap();
        assert!(s.user_for_token(&bob_tok).await.is_some());

        s.delete_user(&bob.id).await.unwrap();
        assert!(s.user_for_token(&bob_tok).await.is_none(), "session must die with the user");
        assert!(s.user_for_token(&admin_tok).await.is_some(), "other sessions unaffected");
    }

    #[tokio::test]
    async fn users_and_hashes_persist_but_are_never_public() {
        let path = temp_path("persist");

        {
            let s = AuthStore::load_at(true, Some(path.clone()), vec![]).unwrap();
            s.setup_first_admin("gorav", "Gorav", "a-good-password").await.unwrap();
        }
        // A new store reads the same file back.
        let s2 = AuthStore::load_at(true, Some(path.clone()), vec![]).unwrap();
        assert!(!s2.setup_required().await);
        assert!(s2.login("gorav", "a-good-password").await.is_ok());

        // The hash is on disk, the plaintext is not...
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("$argon2id$"));
        assert!(!raw.contains("a-good-password"));
        // ...the file is owner-only...
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "auth.json must not be readable by others");
        // ...and the public view carries no credential field at all.
        let public = serde_json::to_string(&s2.list_users().await).unwrap();
        assert!(!public.contains("password"), "public user JSON leaked a field: {public}");

        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn disabled_store_admits_seeded_users_without_setup() {
        let s = AuthStore::load_at(false, None, vec![AuthUser {
            id: "user-1".into(),
            username: "demo".into(),
            full_name: "Demo".into(),
            is_admin: true,
            groups: vec![],
            created_at: "now".into(),
            password_hash: None,
        }])
        .unwrap();
        assert!(!s.enabled);
        assert!(!s.setup_required().await);
        assert_eq!(s.list_users().await.len(), 1);
        // A user with no hash still cannot log in.
        assert!(s.login("demo", "anything123").await.is_err());
    }

    #[test]
    fn group_names_reuse_the_identifier_rules() {
        assert!(validate_group_name("engineers").is_ok());
        for bad in ["", "-root", "has space", &"x".repeat(33)] {
            assert!(validate_group_name(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[tokio::test]
    async fn fresh_store_seeds_the_two_default_groups() {
        let s = temp_store("groups-fresh");
        let names: Vec<_> = s.list_groups().await.into_iter().map(|g| g.name).collect();
        assert!(names.contains(&"admins".to_string()));
        assert!(names.contains(&"family".to_string()));
    }

    #[tokio::test]
    async fn a_legacy_file_without_groups_is_seeded_once_not_every_restart() {
        let path = temp_path("groups-legacy");
        // Simulate an auth.json written before groups existed.
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, r#"{"users":[]}"#).unwrap();

        let s = AuthStore::load_at(true, Some(path.clone()), vec![]).unwrap();
        assert_eq!(s.list_groups().await.len(), 2, "missing key must seed the defaults");
        s.create_group("extra").await.unwrap();

        // Deleting a default group, then restarting, must NOT bring it back —
        // the file now has an explicit (non-empty) group list, so `Some(...)`
        // is respected verbatim rather than falling back to the defaults.
        let admins = s.list_groups().await.into_iter().find(|g| g.name == "admins").unwrap();
        s.delete_group(&admins.id).await.unwrap();
        let after_delete: Vec<_> = s.list_groups().await.into_iter().map(|g| g.name).collect();

        let s2 = AuthStore::load_at(true, Some(path.clone()), vec![]).unwrap();
        let reloaded: Vec<_> = s2.list_groups().await.into_iter().map(|g| g.name).collect();
        assert_eq!(reloaded, after_delete, "a deliberately-emptied default must not reappear");
        assert!(!reloaded.contains(&"admins".to_string()));
        assert!(reloaded.contains(&"extra".to_string()));

        fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[tokio::test]
    async fn group_crud_and_case_insensitive_uniqueness() {
        let s = temp_store("groups-crud");
        let g = s.create_group("Engineers").await.unwrap();
        assert_eq!(g.members.len(), 0);

        assert!(s.create_group("engineers").await.is_err(), "must be case-insensitively unique");
        assert!(s.get_group(&g.id).await.is_some());

        s.delete_group(&g.id).await.unwrap();
        assert!(s.get_group(&g.id).await.is_none());
        assert!(s.delete_group(&g.id).await.is_err(), "deleting twice must not succeed silently");
    }

    #[tokio::test]
    async fn user_creation_rejects_an_unknown_group() {
        let s = temp_store("groups-unknown");
        let err = s.create_user("nell", "Nell", false, vec!["ghosts".into()], Some("a-good-password")).await;
        assert!(matches!(err, Err(ApiError::BadRequest(_))));
    }

    #[tokio::test]
    async fn group_membership_is_computed_live_and_blocks_deletion_while_populated() {
        let s = temp_store("groups-members");
        let g = s.create_group("crew").await.unwrap();
        s.create_user("nell", "Nell", false, vec!["crew".into()], Some("a-good-password")).await.unwrap();

        let refreshed = s.get_group(&g.id).await.unwrap();
        assert_eq!(refreshed.members, vec!["nell".to_string()]);

        // A non-empty group must not be removable out from under its members.
        assert!(s.delete_group(&g.id).await.is_err());
    }
}
