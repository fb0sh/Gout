mod api;
mod config;
mod store;
mod tunnel;
mod web;

use std::net::SocketAddr;


use anyhow::Result;
use clap::Parser;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{error, info, warn};

use config::ServerConfig;
use gout_proto::{STATUS_ERR, STATUS_OK, UDP_FRAME_HEADER};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let config = ServerConfig::parse();

    // 初始化 store
    let store = Arc::new(store::KeyStore::new(&config.data_dir));

    // 首次启动自动生成初始 key
    let init_key = store::ensure_initial_key(&store).await?;
    if !init_key.is_empty() {
        info!("──────────────────────────────────────────");
        info!("  Initial API key: {}", init_key);
        info!("  Save this key! It won't be shown again.");
        info!("──────────────────────────────────────────");
    }

    // 初始化隧道管理器
    let tunnel_mgr = Arc::new(tunnel::TunnelManager::new(
        config.port_start,
        config.port_end,
        // data_port 从 data_addr 解析
        config.data_addr.parse::<SocketAddr>()?.port(),
    ));
    tunnel_mgr.start_cleanup_loop(); // start handshake timeout checker

    // HTTP 服务器（REST API + Web 面板）
    let app = api::build_router(tunnel_mgr.clone(), store.clone());
    let http_addr: SocketAddr = config.http_addr.parse()?;
    let http_listener = tokio::net::TcpListener::bind(http_addr).await?;
    info!("HTTP server listening on http://{}", http_addr);

    // 数据通道服务器
    let data_addr: SocketAddr = config.data_addr.parse()?;
    let data_listener = TcpListener::bind(data_addr).await?;
    info!("Data server listening on {}", data_addr);

    // 双端口并发 + 优雅关闭
    tokio::select! {
        r = axum::serve(http_listener, app) => {
            if let Err(e) = r {
                error!("HTTP server error: {e}");
            }
        }
        r = run_data_server(data_listener, tunnel_mgr.clone()) => {
            if let Err(e) = r {
                error!("Data server error: {e}");
            }
        }
        _ = shutdown_signal() => {
            info!("Shutting down...");
        }
    }

    Ok(())
}

/// 监听 Ctrl+C
async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for ctrl-c");
}

/// 数据通道 accept 循环
async fn run_data_server(
    listener: TcpListener,
    mgr: Arc<tunnel::TunnelManager>,
) -> Result<()> {
    loop {
        let (stream, addr) = match listener.accept().await {
            Ok(c) => c,
            Err(e) => {
                error!("accept error: {e}");
                continue;
            }
        };

        let mgr = mgr.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_data_connection(stream, mgr).await {
                warn!("data connection {}: {:#}", addr, e);
            }
        });
    }
}

/// 处理一条数据通道 TCP 连接
async fn handle_data_connection(
    mut stream: TcpStream,
    mgr: Arc<tunnel::TunnelManager>,
) -> Result<()> {
    // 1. 读握手帧: [token: u64 BE][tunnel_type: u8] = 9 bytes
    let mut handshake_buf = [0u8; 9];
    stream.read_exact(&mut handshake_buf).await?;
    let (token, tunnel_type) = gout_proto::decode_handshake(&handshake_buf);

    // 2. 验证隧道存在
    if !mgr.tunnel_exists(token).await {
        send_error(&mut stream, "unknown token").await?;
        return Ok(());
    }

    match tunnel_type {
        gout_proto::TunnelType::Tcp | gout_proto::TunnelType::Http => {
            handle_tcp_connection(stream, token, mgr).await
        }
        gout_proto::TunnelType::Udp => {
            handle_udp_connection(stream, token, mgr).await
        }
    }
}

/// TCP 隧道：判断是信号通道还是数据通道
async fn handle_tcp_connection(
    mut stream: TcpStream,
    token: u64,
    mgr: Arc<tunnel::TunnelManager>,
) -> Result<()> {
    // 尝试注册为信号通道
    match mgr.register_signal_channel(token).await {
        Ok(mut signal_rx) => {
            // 这是第一条数据连接 → 成为信号通道
            stream.write_all(&[STATUS_OK]).await?;
            info!("signal channel established for tunnel {}", token);

            // 用 split 避免 select! 中的借用冲突
            let (mut reader, mut writer) = stream.split();
            let mut buf = [0u8; 1];

            loop {
                tokio::select! {
                    msg = signal_rx.recv() => {
                        match msg {
                            Some(tunnel::SignalMsg::NewExternalConnection) => {
                                if let Err(e) = writer.write_all(&[gout_proto::SIGNAL_NEW_CONN]).await {
                                    warn!("failed to notify client: {e}");
                                    break;
                                }
                            }
                            Some(tunnel::SignalMsg::Shutdown) | None => {
                                break;
                            }
                        }
                    }
                    r = reader.read(&mut buf) => {
                        match r {
                            Ok(0) | Err(_) => {
                                info!("signal channel closed for tunnel {}", token);
                                break;
                            }
                            Ok(_) => {}
                        }
                    }
                }
            }

            // 清理隧道
            mgr.close_tunnel(token).await.ok();
            info!("tunnel {} closed", token);
            Ok(())
        }
        Err(_) => {
            // 信号通道已存在 → 这是数据连接
            // 取出一个待转发的外部连接
            match mgr.take_pending_conn(token).await {
                Ok(ext_stream) => {
                    stream.write_all(&[STATUS_OK]).await?;
                    info!("data channel established for tunnel {} (pending conn)", token);
                    pipe_bidirectional(stream, ext_stream).await;
                    Ok(())
                }
                Err(e) => {
                    send_error(&mut stream, &format!("no pending connection: {e}")).await?;
                    Ok(())
                }
            }
        }
    }
}

/// UDP 隧道：帧封装的数据转发
async fn handle_udp_connection(
    mut stream: TcpStream,
    token: u64,
    mgr: Arc<tunnel::TunnelManager>,
) -> Result<()> {
    // UDP 不需要信号通道，直接握手确认
    stream.write_all(&[STATUS_OK]).await?;
    info!("UDP data channel established for tunnel {}", token);

    // 简单转发：读帧 → （实际转发逻辑在后续版本）
    // v0.1: 保持连接，检测断开后清理
    let mut header_buf = [0u8; UDP_FRAME_HEADER];
    loop {
        match stream.read_exact(&mut header_buf).await {
            Ok(_n) => {
                let len = gout_proto::decode_udp_header(&header_buf) as usize;
                if len == 0 {
                    // 关闭信号
                    break;
                }
                // 读 payload 并丢弃（v0.1 不做实际转发）
                let mut payload = vec![0u8; len];
                stream.read_exact(&mut payload).await?;
            }
            Err(_) => break,
        }
    }

    mgr.close_tunnel(token).await.ok();
    info!("UDP tunnel {} closed", token);
    Ok(())
}

/// 双向 pipe 两个 TCP stream
async fn pipe_bidirectional(mut a: TcpStream, mut b: TcpStream) {
    let (mut ar, mut aw) = a.split();
    let (mut br, mut bw) = b.split();

    tokio::select! {
        r = tokio::io::copy(&mut ar, &mut bw) => {
            if let Err(e) = r {
                warn!("pipe a→b error: {e}");
            }
        }
        r = tokio::io::copy(&mut br, &mut aw) => {
            if let Err(e) = r {
                warn!("pipe b→a error: {e}");
            }
        }
    }
}

/// 发送错误响应: [STATUS_ERR][err_len: u16 BE][err_msg]
async fn send_error(stream: &mut TcpStream, msg: &str) -> Result<()> {
    let mut resp = vec![STATUS_ERR];
    let msg_bytes = msg.as_bytes();
    resp.extend_from_slice(&(msg_bytes.len() as u16).to_be_bytes());
    resp.extend_from_slice(msg_bytes);
    stream.write_all(&resp).await?;
    Ok(())
}
