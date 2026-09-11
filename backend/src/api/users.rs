//! Users & groups.
//!
//! Dashboard identity (who can log in, their groups) lives in
//! [`AuthStore`](crate::auth::AuthStore) — the source of truth regardless of
//! backend. Real OS/Samba account provisioning is a separate, optional
//! concern handled by the active [`UserOps`](crate::usermgr::UserOps) — the
//! mock by default, or real `useradd`/`groupadd`/`smbpasswd` when
//! `FERROUS_USERS=linux`. This module orchestrates the two: create the
//! dashboard record, then provision the OS side, rolling back on failure so
//! the two can never drift into an inconsistent state (see `usermgr`'s module
//! doc for the exact failure policy).

use axum::{
    extract::{Path, State},
    Json,
};

use crate::auth::middleware::AdminUser;
use crate::auth::AuthRef;
use crate::error::{ApiError, ApiResult};
use crate::models::*;
use crate::usermgr::UserOpsRef;

pub async fn list_users(State(auth): State<AuthRef>) -> Json<Vec<User>> {
    Json(auth.list_users().await)
}

pub async fn create_user(
    _admin: AdminUser,
    State(auth): State<AuthRef>,
    State(ops): State<UserOpsRef>,
    Json(req): Json<CreateUserReq>,
) -> ApiResult<Json<User>> {
    let full_name = if req.full_name.trim().is_empty() {
        req.username.clone()
    } else {
        req.full_name.clone()
    };
    let password = req.password.as_deref().filter(|p| !p.is_empty());
    let user = auth
        .create_user(&req.username, &full_name, req.is_admin, req.groups.clone(), password)
        .await?;

    if let Err(e) = ops.provision_user(&user.username, &req.groups, password).await {
        // The OS/Samba side didn't fully come up — don't leave a dashboard
        // login with no real account behind it (see usermgr's module doc).
        let _ = auth.delete_user(&user.id).await;
        return Err(e);
    }
    Ok(Json(user))
}

pub async fn delete_user(
    AdminUser(actor): AdminUser,
    State(auth): State<AuthRef>,
    State(ops): State<UserOpsRef>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    // Deleting yourself would end your own session mid-request; make it an
    // explicit refusal rather than a confusing logout.
    if actor.id == id {
        return Err(ApiError::Conflict(
            "you cannot delete your own account".into(),
        ));
    }
    let user = auth
        .get_user(&id)
        .await
        .ok_or_else(|| ApiError::NotFound(format!("user {id} not found")))?;

    // Fail closed: if the real account can't be confirmed removed, do not
    // remove the dashboard record either — that would leave an account the
    // dashboard believes is gone but that can still authenticate.
    ops.deprovision_user(&user.username).await?;
    auth.delete_user(&id).await?;
    tracing::info!("users: '{}' deleted user {id}", actor.username);
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn list_groups(State(auth): State<AuthRef>) -> Json<Vec<Group>> {
    Json(auth.list_groups().await)
}

pub async fn create_group(
    _admin: AdminUser,
    State(auth): State<AuthRef>,
    State(ops): State<UserOpsRef>,
    Json(req): Json<CreateGroupReq>,
) -> ApiResult<Json<Group>> {
    let group = auth.create_group(&req.name).await?;
    if let Err(e) = ops.ensure_group(&group.name).await {
        let _ = auth.delete_group(&group.id).await;
        return Err(e);
    }
    Ok(Json(group))
}

pub async fn delete_group(
    _admin: AdminUser,
    State(auth): State<AuthRef>,
    State(ops): State<UserOpsRef>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let group = auth
        .get_group(&id)
        .await
        .ok_or_else(|| ApiError::NotFound(format!("group {id} not found")))?;

    // auth::delete_group already refuses a non-empty group (checked against
    // dashboard users), so that friendly error surfaces before we touch the OS.
    auth.delete_group(&id).await?;

    // An empty Unix group left behind by a failed groupdel isn't a security
    // problem the way an orphaned login account would be — see usermgr's
    // module doc — so this is a warning, not a request failure.
    if let Err(e) = ops.remove_group(&group.name).await {
        tracing::warn!("users: dashboard group '{}' removed but `groupdel` failed: {e}", group.name);
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}
