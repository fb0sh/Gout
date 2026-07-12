//! DataServer — 处理数据通道 TCP 连接。

use std::sync::Arc;

use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{error, info, warn};

use crate::tunnel;
use gout_api::UDP_FRAME_HEADER;

/// 数据通道服务器
pub struct DataServer {
    listener: TcpListener,
    mgr: Arc<tunnel::TunnelManager>,
}

impl DataServer {
    pub fn new(listener: TcpListener, mgr: Arc<tunnel::TunnelManager>) -> Self {
        Self { listener, mgr }
    }

    /// accept 循环
    pub async fn run(&self) -> Result<(), std::io::Error> {
        loop {
            let (stream, addr) = match self.listener.accept().await {
                Ok(c) => c,
                Err(e) => {
                    error!("accept error: {e}");
                    continue;
                }
            };
            let mgr = self.mgr.clone();
            tokio::spawn(async move {
                if let Err(e) = Self::handle_connection(stream, mgr).await {
                    warn!("data connection {}: {:#}", addr, e);
                }
            });
        }
    }

    async fn handle_connection(
        mut stream: TcpStream,
        mgr: Arc<tunnel::TunnelManager>,
    ) -> Result<()> {
        let (token, tunnel_type) = gout_api::data_channel::server_receive_handshake(&mut stream)
            .await
            .map_err(|_| std::io::Error::other("handshake parse failed"))?;

        if !mgr.tunnel_exists(token).await {
            gout_api::data_channel::server_reject(&mut stream, "unknown token").await?;
            return Ok(());
        }

        match tunnel_type {
            gout_api::TunnelType::Tcp | gout_api::TunnelType::Http => {
                Self::handle_tcp(stream, token, mgr).await
            }
            gout_api::TunnelType::Udp => {
                Self::handle_udp(stream, token, mgr).await
            }
        }
    }

    async fn handle_tcp(
        mut stream: TcpStream,
        token: u64,
        mgr: Arc<tunnel::TunnelManager>,
    ) -> Result<()> {
        match mgr.register_signal_channel(token).await {
            Ok(mut signal_rx) => {
                // 信号通道
                gout_api::data_channel::server_accept(&mut stream).await?;
                info!("signal channel established for tunnel {}", token);

                let (mut reader, mut writer) = stream.split();
                let mut buf = [0u8; 1];

                loop {
                    tokio::select! {
                        msg = signal_rx.recv() => {
                            match msg {
                                Some(tunnel::SignalMsg::NewExternalConnection) => {
                                    if let Err(e) = writer.write_all(&[gout_api::SIGNAL_NEW_CONN]).await {
                                        warn!("notify client failed: {e}");
                                        break;
                                    }
                                }
                                Some(tunnel::SignalMsg::Shutdown) | None => break,
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

                mgr.close_tunnel(token).await.ok();
                info!("tunnel {} closed", token);
                Ok(())
            }
            Err(_) => {
                // 数据通道
                match mgr.take_pending_conn(token).await {
                    Ok(ext_stream) => {
                        gout_api::data_channel::server_accept(&mut stream).await?;
                        info!("data channel for tunnel {} (pending conn)", token);
                        gout_api::data_channel::pipe_bidirectional(stream, ext_stream).await;
                        Ok(())
                    }
                    Err(e) => {
                        gout_api::data_channel::server_reject(&mut stream, &format!("no pending conn: {e}")).await?;
                        Ok(())
                    }
                }
            }
        }
    }

    async fn handle_udp(
        mut stream: TcpStream,
        token: u64,
        mgr: Arc<tunnel::TunnelManager>,
    ) -> Result<()> {
        gout_api::data_channel::server_accept(&mut stream).await?;
        info!("UDP data channel established for tunnel {}", token);

        let mut header_buf = [0u8; UDP_FRAME_HEADER];
        loop {
            match stream.read_exact(&mut header_buf).await {
                Ok(_n) => {
                    let len = gout_api::decode_udp_header(&header_buf) as usize;
                    if len == 0 { break; }
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
}
