/// axum Router 组装 + Key 管理 handler。

use axum::{
    extract::{Path, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Redirect},
    routing::{delete, get, post},
    Form, Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;

use crate::store::KeyEntry;
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

    // Key 管理 API → 需要 admin key
    let key_routes = Router::new()
        .route("/", post(create_key).get(list_keys))
        .route("/:key", delete(delete_key))
        .layer(middleware::from_fn(auth::require_admin_key))
        .layer(axum::Extension(store.clone()));

    // 隧道 API → 需要隧道 key
    let tunnel_routes = Router::new()
        .route("/", post(tunnels::create_tunnel))
        .route("/:token", delete(tunnels::delete_tunnel))
        .layer(middleware::from_fn(auth::require_tunnel_key))
        .layer(axum::Extension(store.clone()));

    Router::new()
        // Web 面板（localhost-only，无额外认证）
        .route("/", get(|| async { Redirect::to("/dashboard") }))
        .route("/dashboard", get(web::dashboard))
        .route("/keys", get(web::keys_page).post(web_key_create))
        // API
        .nest("/api/v1/keys", key_routes)
        .nest("/api/v1/tunnels", tunnel_routes)
        .with_state(state)
}

// ━━━ Web 面板表单 handler（localhost-only，跳过 admin key）━━━━

#[derive(Deserialize)]
struct WebKeyCreateForm {
    name: String,
}

async fn web_key_create(
    State(state): State<AppState>,
    Form(form): Form<WebKeyCreateForm>,
) -> impl IntoResponse {
    let api_key = gout_proto::generate_api_key();
    let now = chrono::Utc::now();

    let entry = KeyEntry {
        key: api_key.clone(),
        name: form.name,
        created_at: now.to_rfc3339(),
        admin: false,
    };

    match state.store.add(entry).await {
        Ok(()) => Redirect::to("/keys").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ━━━ Key CRUD API handler（管理员用）━━━━━━━━━━━━━━━━━━━━━━━━━━

async fn create_key(
    State(state): State<AppState>,
    Json(req): Json<CreateKeyRequest>,
) -> impl IntoResponse {
    let api_key = gout_proto::generate_api_key();
    let now = chrono::Utc::now();

    let entry = KeyEntry {
        key: api_key.clone(),
        name: req.name.clone(),
        created_at: now.to_rfc3339(),
        admin: false,
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
            // 只返回非 admin key
            let list: Vec<KeyInfo> = keys
                .into_iter()
                .filter(|k| !k.admin)
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
    // 不允许删除 admin key
    if key.starts_with("sk-") {
        if let Ok(true) = state.store.validate_admin(&key).await {
            return (
                StatusCode::FORBIDDEN,
                Json(ApiResponse::<()>::err("cannot delete admin key")),
            )
                .into_response();
        }
    }

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
