/// axum Router 组装 + Key 管理 handler。

use axum::{
    extract::{Path, State},
    http::StatusCode,
    middleware,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use std::sync::Arc;

use crate::tunnel::TunnelManager;
use crate::web;
use gout_proto::{
    api_ok, ApiResponse, CreateKeyRequest, CreateKeyResponse, KeyInfo,
};

mod auth;
pub(crate) mod tunnels;

use tunnels::AppState;

pub fn build_router(
    tunnel_mgr: Arc<TunnelManager>,
    store: Arc<crate::store::KeyStore>,
) -> Router {
    let state = AppState {
        tunnel_mgr,
        store: store.clone(),
    };

    // Key 管理（无认证 — Web 面板通过 localhost 限制访问）
    let key_routes = Router::new()
        .route("/", post(create_key).get(list_keys))
        .route("/:key", delete(delete_key));

    // 隧道管理（需要 API key 认证）
    let tunnel_routes = Router::new()
        .route("/", post(tunnels::create_tunnel))
        .route("/:token", delete(tunnels::delete_tunnel))
        .layer(middleware::from_fn(auth::require_api_key))
        .layer(axum::Extension(store.clone()));

    Router::new()
        // Web 面板
        .route("/", get(|| async { axum::response::Redirect::to("/dashboard") }))
        .route("/dashboard", get(web::dashboard))
        .route("/keys", get(web::keys_page).post(create_key))
        // API
        .nest("/api/v1/keys", key_routes)
        .nest("/api/v1/tunnels", tunnel_routes)
        .with_state(state)
}

// ━━━ Key CRUD handler ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

async fn create_key(
    State(state): State<AppState>,
    Json(req): Json<CreateKeyRequest>,
) -> impl IntoResponse {
    let api_key = gout_proto::generate_api_key();
    let now = chrono::Utc::now();

    let entry = crate::store::KeyEntry {
        key: api_key.clone(),
        name: req.name.clone(),
        created_at: now.to_rfc3339(),
    };

    match state.store.add(entry).await {
        Ok(()) => (
            StatusCode::CREATED,
            Json(ApiResponse::ok(CreateKeyResponse {
                key: api_key,
                name: req.name,
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::<CreateKeyResponse>::err(e.to_string())),
        )
            .into_response(),
    }
}

async fn list_keys(
    State(state): State<AppState>,
) -> impl IntoResponse {
    match state.store.load().await {
        Ok(keys) => {
            let list: Vec<KeyInfo> = keys
                .into_iter()
                .map(|k| KeyInfo {
                    key: mask_key(&k.key),
                    name: k.name,
                })
                .collect();
            (StatusCode::OK, Json(ApiResponse::ok(list))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::<Vec<KeyInfo>>::err(e.to_string())),
        )
            .into_response(),
    }
}

async fn delete_key(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    match state.store.delete(&key).await {
        Ok(true) => (StatusCode::OK, Json(api_ok())).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::err("key not found")),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::<()>::err(e.to_string())),
        )
            .into_response(),
    }
}

fn mask_key(key: &str) -> String {
    if key.len() <= 12 {
        return key.to_string();
    }
    format!("{}...{}", &key[..6], &key[key.len() - 6..])
}
