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
                                    if let Err(e) = gout_api::data_channel::send_notification(&mut writer).await {
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

        // 标记隧道活跃，防止清理循环误关
        let _ = mgr.mark_connected(token).await;

        let udp_socket = match mgr.get_udp_socket(token).await {
            Some(s) => s,
            None => {
                warn!("UDP socket not found for tunnel {}", token);
                return Ok(());
            }
        };

        // tokio::io::split 消费 stream 返回独立 owned 半部，可 move 到不同任务
        let (mut reader, mut writer) = tokio::io::split(stream);

        // 记录外部 peer 地址（第一个数据报决定）
        let peer = Arc::new(std::sync::Mutex::new(None::<std::net::SocketAddr>));
        let peer2 = peer.clone();

        // 子任务：UdpSocket → TCP 数据通道
        // 克隆 Arc<UdpSocket> 以便主循环也能使用
        let udp_tx = udp_socket.clone();
        let send_handle = tokio::spawn(async move {
            let mut buf = vec![0u8; 65535];
            loop {
                let (len, addr) = match udp_tx.recv_from(&mut buf).await {
                    Ok(n) => n,
                    Err(_) => break,
                };
                *peer2.lock().unwrap() = Some(addr);
                let frame = gout_api::encode_udp_frame(&buf[..len]);
                if writer.write_all(&frame).await.is_err() {
                    break;
                }
            }
        });

        // 主循环：TCP 数据通道 → UdpSocket
        let mut header_buf = [0u8; UDP_FRAME_HEADER];
        loop {
            match reader.read_exact(&mut header_buf).await {
                Ok(_) => {
                    let len = gout_api::decode_udp_header(&header_buf) as usize;
                    if len == 0 { break; }
                    let mut payload = vec![0u8; len];
                    reader.read_exact(&mut payload).await?;
                    // 如果已有 peer 地址则转发，否则丢弃（尚无外部数据来源）
                    // 先复制出 peer 地址再 .await，避免 MutexGuard 跨越 await
                    let peer_addr = *peer.lock().unwrap();
                    if let Some(addr) = peer_addr {
                        udp_socket.send_to(&payload, addr).await.ok();
                    }
                }
                Err(_) => break,
            }
        }

        send_handle.abort();
        mgr.close_tunnel(token).await.ok();
        info!("UDP tunnel {} closed", token);
        Ok(())
    }
}
