//! 隧道会话 — 管理一条隧道的完整生命周期。

use std::sync::Arc;
use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

use gout_api::{TunnelType, UDP_FRAME_HEADER};

/// 解析域名到 IP，解析失败则返回原域名。
async fn resolve_host(host: &str) -> String {
    tokio::net::lookup_host(format!("{host}:0"))
        .await
        .ok()
        .and_then(|mut addrs| addrs.next())
        .map(|sa| sa.ip().to_string())
        .unwrap_or_else(|| host.to_string())
}

/// 隧道会话：REST 创建 → 握手 → 信号循环 → 数据转发 → 清理
pub struct TunnelSession {
    pub token: u64,
    pub tunnel_type: TunnelType,
    pub local_port: u16,
    server: crate::config::ServerConfig,
}

/// 返回一个 future：前台等待 Ctrl+C，后台永不 resolve。
async fn ctrl_c_signal() {
    if std::env::var("GOUT_DAEMON_PIDFILE").is_ok() {
        // 后台模式：没有控制终端，永不触发
        std::future::pending().await
    } else {
        tokio::signal::ctrl_c().await.ok();
    }
}

impl TunnelSession {
    /// 通过 REST API 创建隧道，建立信号通道。
    ///
    /// 如果环境变量 `GOUT_DAEMON_TOKEN` 存在（由 `-d` 的父进程设置），
    /// 则跳过 REST API 创建，直接使用传入的 token 进行握手。
    pub async fn create(server: crate::config::ServerConfig, tunnel_type: TunnelType, local_port: u16, remote_port: Option<u16>) -> Result<Self> {
        let server_host = resolve_host(
            server.addr.split(':').next().unwrap_or(&server.addr)
        ).await;
        // REST API 地址也用 IP（避免重复 DNS）
        let server_port = server.addr.split(':').nth(1).unwrap_or("8080");
        let resolved_addr = format!("{}:{}", server_host, server_port);

        // 检查是否由父进程（-d）预先创建了隧道
        let (token, data_port) = if let (Ok(t), Ok(dp)) = (
            std::env::var("GOUT_DAEMON_TOKEN"),
            std::env::var("GOUT_DAEMON_DATA_PORT"),
        ) {
            let t: u64 = t.parse()?;
            let dp: u16 = dp.parse()?;
            println!("[+] {} tunnel: 127.0.0.1:{} -> {}:{}",
                tunnel_type, local_port, server_host, "?");
            (t, dp)
        } else {
            // 正常模式：通过 REST API 创建
            let gout = gout_api::client::GoutClient::new(&resolved_addr, &server.api_key);
            let tunnel = gout.create_tunnel(tunnel_type, local_port, remote_port).await?;

            let local_url = if tunnel_type == TunnelType::Http {
                format!("http://127.0.0.1:{}", local_port)
            } else {
                format!("127.0.0.1:{}", local_port)
            };
            let remote_url = if tunnel_type == TunnelType::Http {
                format!("http://{}:{}", server_host, tunnel.public_port)
            } else {
                format!("{}:{}", server_host, tunnel.public_port)
            };
            println!("[+] {} tunnel: {} -> {}", tunnel_type, local_url, remote_url);

            (tunnel.token, tunnel.data_port)
        };

        // 连接数据端口 + 握手
        let data_addr = format!("{}:{}", server_host, data_port);
        let mut stream = TcpStream::connect(&data_addr)
            .await
            .context("connect to data port failed")?;

        gout_api::data_channel::client_handshake(
            &mut stream,
            token,
            tunnel_type,
        ).await.context("handshake failed")?;

        match tunnel_type {
            TunnelType::Udp => {
                println!("[+] UDP data channel established, forwarding...");
                println!("    Ctrl+C to close tunnel");
                Self::run_udp_data_loop(stream, local_port).await?;
            }
            _ => {
                println!("[+] signal channel established, waiting for connections...");
                println!("    Ctrl+C to close tunnel");
                Self::run_signal_loop(stream, &server, token, tunnel_type, local_port, data_port).await?;
            }
        }

        // 清理
        println!("[-] closing tunnel...");
        let gout = gout_api::client::GoutClient::new(&resolved_addr, &server.api_key);
        gout.delete_tunnel(token).await.ok();

        Ok(Self {
            token,
            tunnel_type,
            local_port,
            server,
        })
    }

    /// 信号循环：等待服务端通知 → 对每个新连接建立数据通道
    async fn run_signal_loop(
        mut stream: TcpStream,
        server: &crate::config::ServerConfig,
        token: u64,
        tunnel_type: TunnelType,
        local_port: u16,
        data_port: u16,
    ) -> Result<()> {
        loop {
            tokio::select! {
                sig = gout_api::data_channel::read_signal(&mut stream) => {
                    match sig {
                        gout_api::data_channel::SignalKind::NewConnection => {
                            let sc = server.clone();
                            let dp = data_port;
                            tokio::spawn(async move {
                                Self::handle_data_channel(sc, token, tunnel_type, local_port, dp).await;
                            });
                        }
                        gout_api::data_channel::SignalKind::Disconnected => break,
                    }
                }
                _ = ctrl_c_signal() => {
                    println!("[-] closing tunnel...");
                    break;
                }
            }
        }
        Ok(())
    }

    /// UDP 数据循环：双向转发（TCP 数据通道 ↔ localhost UDP）
    async fn run_udp_data_loop(
        stream: TcpStream,
        local_port: u16,
    ) -> Result<()> {
        let local = Arc::new(UdpSocket::bind("127.0.0.1:0").await?);
        local.connect(format!("127.0.0.1:{local_port}")).await?;

        let (mut reader, mut writer) = tokio::io::split(stream);

        // 子任务：TCP → local UDP
        let local_tx = local.clone();
        let recv_handle = tokio::spawn(async move {
            let mut header_buf = [0u8; UDP_FRAME_HEADER];
            loop {
                match reader.read_exact(&mut header_buf).await {
                    Ok(_) => {
                        let len = gout_api::decode_udp_header(&header_buf) as usize;
                        if len == 0 { break; }
                        let mut payload = vec![0u8; len];
                        if reader.read_exact(&mut payload).await.is_err() { break; }
                        if local_tx.send(&payload).await.is_err() { break; }
                    }
                    Err(_) => break,
                }
            }
        });

        // 主循环：local UDP → TCP
        let mut recv_buf = vec![0u8; 65535];
        loop {
            tokio::select! {
                r = local.recv(&mut recv_buf) => {
                    match r {
                        Ok(len) => {
                            let frame = gout_api::encode_udp_frame(&recv_buf[..len]);
                            if writer.write_all(&frame).await.is_err() { break; }
                        }
                        Err(_) => break,
                    }
                }
                _ = ctrl_c_signal() => {
                    println!("[-] closing tunnel...");
                    break;
                }
            }
        }

        // 发送空帧通知服务端关闭
        let _ = writer.write_all(&[0, 0]).await;
        recv_handle.abort();
        Ok(())
    }

    /// 处理一条外部连接：数据通道 → localhost pipe
    async fn handle_data_channel(
        server: crate::config::ServerConfig,
        token: u64,
        tunnel_type: TunnelType,
        local_port: u16,
        data_port: u16,
    ) {
        let server_host = resolve_host(
            server.addr.split(':').next().unwrap_or(&server.addr)
        ).await;
        let data_addr = format!("{server_host}:{data_port}");
        let mut stream = match TcpStream::connect(&data_addr).await {
            Ok(s) => s,
            Err(e) => { eprintln!("[-] connect data port failed: {e}"); return; }
        };

        if gout_api::data_channel::client_handshake(&mut stream, token, tunnel_type).await.is_err() {
            eprintln!("[-] data channel handshake rejected");
            return;
        }

        let local = match TcpStream::connect(format!("127.0.0.1:{local_port}")).await {
            Ok(s) => s,
            Err(_) => {
                eprintln!("[-] localhost:{local_port} not reachable — service not running?");
                return;
            }
        };

        gout_api::data_channel::pipe_bidirectional(stream, local).await;
    }
}
