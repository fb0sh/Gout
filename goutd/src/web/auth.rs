//! Web 面板认证中间件 — 检查 gout_admin_session cookie。

use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use std::sync::Arc;

use crate::store::KeyStore;

pub async fn require_web_auth(
    req: Request,
    next: Next,
) -> Result<Response, Response> {
    // 检查 cookie
    let cookie = req
        .headers()
        .get("Cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|c| {
            c.split(';')
                .filter_map(|pair| {
                    let mut parts = pair.trim().splitn(2, '=');
                    let key = parts.next()?;
                    let val = parts.next()?;
                    if key == "gout_admin_session" { Some(val.to_string()) } else { None }
                })
                .next()
        });

    let admin_key = match cookie {
        Some(k) => k,
        None => return Err(Redirect::to("/login").into_response()),
    };

    // 验证 admin key
    let store = req
        .extensions()
        .get::<Arc<KeyStore>>()
        .cloned()
        .ok_or_else(|| {
            (StatusCode::INTERNAL_SERVER_ERROR, "store not found").into_response()
        })?;

    match store.validate_admin(&admin_key).await {
        Ok(true) => Ok(next.run(req).await),
        _ => Err(Redirect::to("/login").into_response()),
    }
}
