/// Web 面板 handler。

use askama::Template;
use axum::{extract::State, response::Html};

use super::api::tunnels::AppState;
use crate::store::KeyEntry;
use crate::tunnel::TunnelInfo;

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    active_page: &'static str,
    tunnels: Vec<TunnelViewModel>,
}

#[derive(Template)]
#[template(path = "keys.html")]
struct KeysTemplate {
    active_page: &'static str,
    keys: Vec<KeyViewModel>,
}

struct TunnelViewModel {
    token: u64,
    tunnel_type: String,
    public_port: u16,
    key_name: String,
    has_signal: bool,
    pending_count: usize,
}

struct KeyViewModel {
    key: String,
    name: String,
}

pub async fn dashboard(
    State(state): State<AppState>,
) -> Result<Html<String>, (axum::http::StatusCode, String)> {
    let tunnels: Vec<TunnelViewModel> = state
        .tunnel_mgr
        .list_tunnels()
        .await
        .into_iter()
        .map(|t: TunnelInfo| TunnelViewModel {
            token: t.token,
            tunnel_type: t.tunnel_type.as_str().to_string(),
            public_port: t.public_port,
            key_name: t.key_name,
            has_signal: t.has_signal,
            pending_count: t.pending_count,
        })
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
    let keys: Vec<KeyViewModel> = state
        .store
        .load()
        .await
        .map_err(|e: anyhow::Error| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .into_iter()
        .filter(|k: &KeyEntry| !k.admin)
        .map(|k: KeyEntry| KeyViewModel {
            key: mask_key(&k.key),
            name: k.name,
        })
        .collect();

    let tmpl = KeysTemplate {
        active_page: "keys",
        keys,
    };

    tmpl.render()
        .map(Html)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

fn mask_key(key: &str) -> String {
    if key.len() <= 12 {
        return key.to_string();
    }
    format!("{}...{}", &key[..6], &key[key.len() - 6..])
}
