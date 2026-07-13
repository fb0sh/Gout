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

/// 角色标记
enum KeyRole { Admin, Tunnel }

/// 共享认证逻辑
async fn require_key(
    req: Request,
    next: Next,
    header: &'static str,
    role: KeyRole,
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
        .get(header)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if key.is_empty() {
        return Err((StatusCode::UNAUTHORIZED, format!("missing {header}")).into_response());
    }

    let valid = match role {
        KeyRole::Admin => store.validate_admin(key).await.unwrap_or(false),
        KeyRole::Tunnel => store.validate_tunnel(key).await.unwrap_or(false),
    };

    if valid {
        Ok(next.run(req).await)
    } else {
        Err((StatusCode::UNAUTHORIZED, format!("invalid {header}")).into_response())
    }
}

/// Admin key 验证（管理面板 + key 管理）
pub async fn require_admin_key(
    req: Request,
    next: Next,
) -> Result<Response, Response> {
    require_key(req, next, "X-Admin-Key", KeyRole::Admin).await
}

/// 隧道 key 验证（tunnel CRUD）
pub async fn require_tunnel_key(
    req: Request,
    next: Next,
) -> Result<Response, Response> {
    require_key(req, next, "X-Api-Key", KeyRole::Tunnel).await
}
