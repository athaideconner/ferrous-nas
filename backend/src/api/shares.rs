//! SMB / NFS shares. Mutations are handled by the active
//! [`ShareManager`](crate::sharemgr) — the mock store by default, or real
//! Samba/NFS config generation when `FERROUS_SHARES=linux`.

use axum::{
    extract::{Path, State},
    Json,
};

use crate::auth::middleware::AdminUser;
use crate::error::ApiResult;
use crate::models::*;
use crate::sharemgr::ShareManagerRef;

pub async fn list_shares(State(mgr): State<ShareManagerRef>) -> ApiResult<Json<Vec<Share>>> {
    Ok(Json(mgr.list().await?))
}

pub async fn create_share(
    _admin: AdminUser,
    State(mgr): State<ShareManagerRef>,
    Json(req): Json<CreateShareReq>,
) -> ApiResult<Json<Share>> {
    Ok(Json(mgr.create(req).await?))
}

pub async fn patch_share(
    _admin: AdminUser,
    State(mgr): State<ShareManagerRef>,
    Path(id): Path<String>,
    Json(req): Json<PatchShareReq>,
) -> ApiResult<Json<Share>> {
    Ok(Json(mgr.patch(&id, req).await?))
}

pub async fn delete_share(
    _admin: AdminUser,
    State(mgr): State<ShareManagerRef>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    mgr.delete(&id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}
