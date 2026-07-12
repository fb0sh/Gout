/// API key 验证中间件。
///
/// 两个中间件共享同一个 `Arc<KeyStore>`（通过 Extension 注入）：
/// - `require_admin_key`  — 检查 `X-Admin-Key`，仅 admin key 可用
/// - `require_tunnel_key` — 检查 `X-Api-Key`，仅普通隧道 key 可用

use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

use crate::store::KeyStore;

/// Admin key 验证（管理面板 + key 管理）
pub async fn require_admin_key(
    req: Request,
    next: Next,
) -> Result<Response, Response> {
    let store = req
        .extensions()
        .get::<Arc<KeyStore>>()
        .cloned()
        .ok_or_else(|| {
            (StatusCode::INTERNAL_SERVER_ERROR, "store not found").into_response()
        })?;

    let key = req
        .headers()
        .get("X-Admin-Key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if key.is_empty() {
        return Err((StatusCode::UNAUTHORIZED, "missing X-Admin-Key header").into_response());
    }

    match store.validate_admin(key).await {
        Ok(true) => Ok(next.run(req).await),
        _ => Err((StatusCode::UNAUTHORIZED, "invalid admin key").into_response()),
    }
}

/// 隧道 key 验证（tunnel CRUD）
pub async fn require_tunnel_key(
    req: Request,
    next: Next,
) -> Result<Response, Response> {
    let store = req
        .extensions()
        .get::<Arc<KeyStore>>()
        .cloned()
        .ok_or_else(|| {
            (StatusCode::INTERNAL_SERVER_ERROR, "store not found").into_response()
        })?;

    let key = req
        .headers()
        .get("X-Api-Key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if key.is_empty() {
        return Err((StatusCode::UNAUTHORIZED, "missing X-Api-Key header").into_response());
    }

    match store.validate_tunnel(key).await {
        Ok(true) => Ok(next.run(req).await),
        _ => Err((StatusCode::UNAUTHORIZED, "invalid API key").into_response()),
    }
}
