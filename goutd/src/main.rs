mod api;
mod config;
mod data_server;
mod store;
mod tunnel;
mod web;

use std::net::SocketAddr;

use anyhow::Result;
use clap::Parser;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{error, info};

use config::ServerConfig;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    let config = ServerConfig::parse();

    // 初始化 store
    let store = Arc::new(store::KeyStore::new(&config.data_dir));

    // 首次启动自动生成初始 key
    let init_key = store::ensure_initial_key(&store).await?;
    if !init_key.is_empty() {
        info!("──────────────────────────────────────────");
        info!("  Admin API Key: {}", init_key);
        info!("  Save this key! It won't be shown again.");
        info!("──────────────────────────────────────────");
    }

    // 初始化隧道管理器
    let tunnel_mgr = Arc::new(tunnel::TunnelManager::new(
        config.port_start,
        config.port_end,
        config.data_addr.parse::<SocketAddr>()?.port(),
    ));
    tunnel_mgr.start_cleanup_loop();

    // HTTP 服务器（REST API + Web 面板）
    let app = api::build_router(tunnel_mgr.clone(), store.clone());
    let http_addr: SocketAddr = config.http_addr.parse()?;
    let http_listener = TcpListener::bind(http_addr).await?;
    info!("HTTP server listening on http://{}", http_addr);

    // 数据通道服务器
    let data_addr: SocketAddr = config.data_addr.parse()?;
    let data_listener = TcpListener::bind(data_addr).await?;
    info!("Data server listening on {}", data_addr);

    let data_srv = data_server::DataServer::new(data_listener, tunnel_mgr.clone());

    // 双端口并发 + 优雅关闭
    tokio::select! {
        r = axum::serve(http_listener, app) => {
            if let Err(e) = r { error!("HTTP server error: {e}"); }
        }
        r = data_srv.run() => {
            if let Err(e) = r { error!("Data server error: {e}"); }
        }
        _ = shutdown_signal() => {
            info!("Shutting down...");
        }
    }

    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for ctrl-c");
}
