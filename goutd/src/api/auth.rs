/// API key 验证中间件 — 从 request extensions 读取 KeyStore。

use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

use crate::store::KeyStore;

pub async fn require_api_key(
    req: Request,
    next: Next,
) -> Result<Response, Response> {
    let store = req
        .extensions()
        .get::<Arc<KeyStore>>()
        .cloned()
        .ok_or_else(|| {
            (StatusCode::INTERNAL_SERVER_ERROR, "store not found (bug: missing Extension layer)").into_response()
        })?;

    let key = req
        .headers()
        .get("X-Api-Key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if key.is_empty() {
        return Err((
            StatusCode::UNAUTHORIZED,
            "missing X-Api-Key header",
        )
            .into_response());
    }

    match store.validate(key).await {
        Ok(true) => Ok(next.run(req).await),
        Ok(false) => Err((
            StatusCode::UNAUTHORIZED,
            "invalid API key",
        )
            .into_response()),
        Err(_) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to validate key",
        )
            .into_response()),
    }
}
