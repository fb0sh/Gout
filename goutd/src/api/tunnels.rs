/// 隧道 CRUD handler。
///
/// POST   /api/v1/tunnels  — 创建隧道
/// DELETE /api/v1/tunnels/:token — 删除隧道

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;

use crate::tunnel::TunnelManager;
use gout_api::{ApiResponse, CreateTunnelRequest, TunnelResponse};

#[derive(Clone)]
pub struct AppState {
    pub tunnel_mgr: Arc<TunnelManager>,
    pub store: Arc<crate::store::KeyStore>,
}

pub async fn create_tunnel(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CreateTunnelRequest>,
) -> impl IntoResponse {
    // 提取 API key 获取 key_name
    let api_key = headers
        .get("X-Api-Key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let key_name = match state.store.find_name(api_key).await {
        Ok(Some(name)) => name,
        _ => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiResponse::<TunnelResponse>::err("invalid API key")),
            )
                .into_response();
        }
    };

    match state.tunnel_mgr.create_tunnel(req.tunnel_type, key_name, "0.0.0.0".parse().unwrap()).await {
        Ok((token, port)) => {
            let resp = TunnelResponse {
                token,
                public_port: port,
                data_port: state.tunnel_mgr.data_port(),
                tunnel_type: req.tunnel_type.as_str().to_string(),
            };
            (
                StatusCode::CREATED,
                Json(ApiResponse::ok(resp)),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiResponse::<TunnelResponse>::err(e)),
        )
            .into_response(),
    }
}

pub async fn delete_tunnel(
    State(state): State<AppState>,
    Path(token): Path<u64>,
) -> impl IntoResponse {
    match state.tunnel_mgr.close_tunnel(token).await {
        Ok(()) => (
            StatusCode::OK,
            Json(gout_api::api_ok()),
        )
            .into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::err(e)),
        )
            .into_response(),
    }
}
