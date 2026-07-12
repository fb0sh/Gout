//! 隧道会话 — 管理一条隧道的完整生命周期。

use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use gout_api::TunnelType;

/// 隧道会话：REST 创建 → 握手 → 信号循环 → 数据转发 → 清理
pub struct TunnelSession {
    pub token: u64,
    pub tunnel_type: TunnelType,
    pub local_port: u16,
    config: crate::config::Config,
}

impl TunnelSession {
    /// 通过 REST API 创建隧道，建立信号通道
    pub async fn create(config: crate::config::Config, tunnel_type: TunnelType, local_port: u16) -> Result<Self> {
        let gout = gout_api::client::GoutClient::new(&config.server.addr, &config.server.api_key);
        let tunnel = gout.create_tunnel(tunnel_type, local_port).await?;

        println!("✅ 隧道已创建");
        println!("   公网端口: {}  →  localhost:{}", tunnel.public_port, local_port);
        println!("   数据端口: {}", tunnel.data_port);

        // 连接数据端口 + 握手
        let data_addr = format!("{}:{}", server_host(&config.server.addr), tunnel.data_port);
        let mut stream = TcpStream::connect(&data_addr)
            .await
            .context("connect to data port failed")?;

        gout_api::data_channel::client_handshake(
            &mut stream,
            tunnel.token,
            tunnel_type,
        ).await.context("handshake failed")?;

        println!("   信号通道已建立，等待外部连接...");
        println!("   隧道已就绪！");

        // 进入信号循环
        Self::run_signal_loop(stream, &config, tunnel.token, tunnel_type, local_port).await?;

        // 清理
        gout.delete_tunnel(tunnel.token).await.ok();
        println!("隧道已关闭");

        Ok(Self {
            token: tunnel.token,
            tunnel_type,
            local_port,
            config,
        })
    }

    /// 信号循环：等待服务端通知 → 对每个新连接建立数据通道
    async fn run_signal_loop(
        mut stream: TcpStream,
        config: &crate::config::Config,
        token: u64,
        tunnel_type: TunnelType,
        local_port: u16,
    ) -> Result<()> {
        println!("   按 Ctrl+C 关闭隧道");

        let mut buf = [0u8; 1];
        loop {
            tokio::select! {
                r = stream.read(&mut buf) => {
                    match r {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            if buf[0] == gout_api::SIGNAL_NEW_CONN {
                                let config = config.clone();
                                tokio::spawn(async move {
                                    Self::handle_data_channel(config, token, tunnel_type, local_port).await;
                                });
                            }
                        }
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    println!("\n正在关闭隧道...");
                    break;
                }
            }
        }
        Ok(())
    }

    /// 处理一条外部连接：数据通道 → localhost pipe
    async fn handle_data_channel(
        config: crate::config::Config,
        token: u64,
        tunnel_type: TunnelType,
        local_port: u16,
    ) {
        let data_addr = format!("{}:8081", server_host(&config.server.addr));
        let mut stream = match TcpStream::connect(&data_addr).await {
            Ok(s) => s,
            Err(e) => { eprintln!("connect data port failed: {e}"); return; }
        };

        if gout_api::data_channel::client_handshake(&mut stream, token, tunnel_type).await.is_err() {
            eprintln!("data channel handshake rejected");
            return;
        }

        let local = match TcpStream::connect(format!("127.0.0.1:{local_port}")).await {
            Ok(s) => s,
            Err(_) => {
                eprintln!("连接 localhost:{local_port} 失败 — 本地服务未启动？");
                return;
            }
        };

        gout_api::data_channel::pipe_bidirectional(stream, local).await;
    }
}

fn server_host(addr: &str) -> &str {
    addr.split(':').next().unwrap_or(addr)
}
