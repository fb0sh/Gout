/// Web 面板 handler。

use askama::Template;
use axum::{extract::State, response::{Html, IntoResponse}};

use super::api::tunnels::AppState;
use crate::store::KeyEntry;
use crate::tunnel::TunnelInfo;
use gout_api::TunnelListEntry;

pub(crate) mod auth;

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    active_page: &'static str,
    tunnels: Vec<TunnelListEntry>,
}

#[derive(Template)]
#[template(path = "keys.html")]
struct KeysTemplate {
    active_page: &'static str,
    keys: Vec<KeyViewModel>,
    admin_key: String,
}

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate {
    error: Option<String>,
}

struct KeyViewModel {
    key: String,
    name: String,
}

/// 登录页面
pub async fn login_page() -> Html<String> {
    let tmpl = LoginTemplate { error: None };
    Html(tmpl.render().unwrap_or_default())
}

/// 登录表单提交
pub async fn login_post(
    State(state): State<AppState>,
    axum::Form(form): axum::Form<LoginForm>,
) -> axum::response::Response {
    let is_valid = state.store.validate_admin(&form.key).await.unwrap_or(false);
    if !is_valid {
        let tmpl = LoginTemplate { error: Some("无效的 admin key".into()) };
        return (axum::http::StatusCode::UNAUTHORIZED, Html(tmpl.render().unwrap_or_default())).into_response();
    }

    let cookie = format!("gout_admin_session={}; Max-Age=86400; Path=/; HttpOnly", form.key);
    (
        [(axum::http::header::SET_COOKIE, cookie)],
        axum::response::Redirect::to("/dashboard"),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
pub struct LoginForm {
    key: String,
}

pub async fn logout() -> impl axum::response::IntoResponse {
    (
        [(axum::http::header::SET_COOKIE, "gout_admin_session=; Max-Age=0; Path=/")],
        axum::response::Redirect::to("/login"),
    )
}

pub async fn dashboard(
    State(state): State<AppState>,
) -> Result<Html<String>, (axum::http::StatusCode, String)> {
    let tunnels: Vec<TunnelListEntry> = state
        .tunnel_mgr
        .list_tunnels()
        .await
        .into_iter()
        .map(|t: TunnelInfo| t.to_list_entry())
        .collect();

    let tmpl = DashboardTemplate {
        active_page: "dashboard",
        tunnels,
    };

    tmpl.render()
        .map(Html)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn keys_page(
    State(state): State<AppState>,
) -> Result<Html<String>, (axum::http::StatusCode, String)> {
    let admin_key = state
        .store
        .find_admin_key()
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .unwrap_or_default();

    let keys: Vec<KeyViewModel> = state
        .store
        .load()
        .await
        .map_err(|e: anyhow::Error| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .into_iter()
        .filter(|k: &KeyEntry| !k.admin)
        .map(|k: KeyEntry| KeyViewModel {
            key: k.key.clone(),
            name: k.name,
        })
        .collect();

    let tmpl = KeysTemplate {
        active_page: "keys",
        keys,
        admin_key,
    };

    tmpl.render()
        .map(Html)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}


