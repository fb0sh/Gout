mod cli;
mod config;

use anyhow::{Context, Result};
use gout_proto::{ApiResponse, CreateTunnelRequest, TunnelResponse};

fn main() -> Result<()> {
    // 设置 tracing（默认只输出 error 级别）
    tracing_subscriber::fmt().init();
    cli::Cli::run()
}

/// 处理 `login` 命令
fn cmd_login(server: &str, key: &str) -> Result<()> {
    config::write(server, key)?;
    println!("✅ 凭据已保存到 ~/.goutrc");
    println!("   服务器: {server}");
    println!("   使用方式: gout tcp <port>");
    Ok(())
}

/// 处理 `tcp/udp/http` 命令
fn cmd_tunnel(tunnel_type: &str, local_port: u16) -> Result<()> {
    let cfg = config::read()?;

    println!("🔗 创建 {tunnel_type} 隧道 {local_port} → {}", cfg.server.addr);

    // 这里用 tokio runtime 来执行异步 REST 调用
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        // 1. REST 创建隧道
        let client = reqwest::Client::new();
        let create_url = format!("http://{}/api/v1/tunnels", cfg.server.addr);

        let resp = client
            .post(&create_url)
            .header("X-Api-Key", &cfg.server.api_key)
            .json(&CreateTunnelRequest {
                tunnel_type: match tunnel_type {
                    "tcp" => gout_proto::TunnelType::Tcp,
                    "udp" => gout_proto::TunnelType::Udp,
                    "http" => gout_proto::TunnelType::Http,
                    _ => unreachable!(),
                },
                local_port: Some(local_port),
            })
            .send()
            .await
            .context("REST create tunnel failed")?;

        if !resp.status().is_success() {
            let api_resp: ApiResponse<TunnelResponse> = resp
                .json()
                .await
                .context("parse error response")?;
            anyhow::bail!("server error: {}", api_resp.error.unwrap_or_default());
        }

        let api_resp: ApiResponse<TunnelResponse> = resp
            .json()
            .await
            .context("parse success response")?;
        let tunnel = api_resp.data.context("no tunnel data in response")?;

        println!("✅ 隧道已创建");
        println!("   公网端口: {}  →  localhost:{}", tunnel.public_port, local_port);
        println!("   数据端口: {}", tunnel.data_port);
        println!("   按 Ctrl+C 关闭隧道");

        // 2. 连接数据端口 + 握手（信号通道）
        let data_addr = format!("{}:{}", server_host(&cfg.server.addr), tunnel.data_port);
        let mut stream = tokio::net::TcpStream::connect(&data_addr)
            .await
            .context("connect to data port failed")?;

        // 握手：发送 [token: u64 BE][tunnel_type: u8]
        let handshake = gout_proto::encode_handshake(tunnel.token, parse_tt(tunnel_type));
        stream
            .write_all(&handshake)
            .await
            .context("send handshake failed")?;

        // 读响应
        let mut status = [0u8; 1];
        stream.read_exact(&mut status).await?;
        if status[0] != gout_proto::STATUS_OK {
            anyhow::bail!("handshake rejected by server");
        }

        println!("   信号通道已建立，等待外部连接...");
        println!("   隧道已就绪！");

        // 3. 信号通道循环 + 数据转发
        if tunnel_type == "udp" {
            run_udp_channel(stream, tunnel, local_port, cfg).await?;
        } else {
            run_tcp_signal_channel(stream, tunnel, local_port, cfg).await?;
        }

        Ok(())
    })
}

/// TCP 信号通道循环
async fn run_tcp_signal_channel(
    mut stream: tokio::net::TcpStream,
    tunnel: TunnelResponse,
    local_port: u16,
    cfg: config::Config,
) -> Result<()> {
    let mut buf = [0u8; 1];
    loop {
        tokio::select! {
            r = stream.read(&mut buf) => {
                match r {
                    Ok(0) | Err(_) => {
                        println!("信号通道已关闭");
                        break;
                    }
                    Ok(_) => {
                        if buf[0] == gout_proto::SIGNAL_NEW_CONN {
                            tokio::spawn(handle_data_connection(
                                tunnel.token,
                                tunnel.tunnel_type.clone(),
                                local_port,
                                cfg.clone(),
                            ));
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

    // 清理
    let client = reqwest::Client::new();
    let del_url = format!("http://{}/api/v1/tunnels/{}", cfg.server.addr, tunnel.token);
    let _ = client
        .delete(&del_url)
        .header("X-Api-Key", &cfg.server.api_key)
        .send()
        .await;

    println!("隧道已关闭");
    Ok(())
}

/// 处理一条外部连接：连接数据通道 → 连接 localhost → pipe
async fn handle_data_connection(
    token: u64,
    tunnel_type: String,
    local_port: u16,
    cfg: config::Config,
) {
    // 从配置中解析 server_host 和数据端口
    // 隧道创建时返回了 data_port，但我们没有保存。重新解析或使用默认
    // 优先使用 config 中的 addr 和默认 data_port
    let data_addr = format!("{}:8081", server_host(&cfg.server.addr));

    let mut stream = match tokio::net::TcpStream::connect(&data_addr).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("connect to data port failed: {}", e);
            return;
        }
    };

    // 握手
    let handshake = gout_proto::encode_handshake(token, parse_tt(&tunnel_type));
    if stream.write_all(&handshake).await.is_err() {
        return;
    }

    let mut status = [0u8; 1];
    if stream.read_exact(&mut status).await.is_err() || status[0] != gout_proto::STATUS_OK {
        eprintln!("data channel handshake rejected");
        return;
    }

    // 连接 localhost
    let mut local = match tokio::net::TcpStream::connect(format!("127.0.0.1:{}", local_port)).await {
        Ok(s) => s,
        Err(_) => {
            // 本地服务未启动，通知服务端
            let _ = stream.write_all(&[0u8; 1]).await;
            eprintln!("连接 localhost:{} 失败 — 本地服务未启动？", local_port);
            return;
        }
    };

    // pipe
    let (mut sr, mut sw) = stream.split();
    let (mut lr, mut lw) = local.split();
    tokio::select! {
        _ = tokio::io::copy(&mut sr, &mut lw) => {}
        _ = tokio::io::copy(&mut lr, &mut sw) => {}
    }
}

/// UDP 通道
async fn run_udp_channel(
    mut stream: tokio::net::TcpStream,
    _tunnel: TunnelResponse,
    _local_port: u16,
    _cfg: config::Config,
) -> Result<()> {
    let mut buf = [0u8; gout_proto::UDP_FRAME_HEADER];
    // 保持连接，等待关闭
    loop {
        tokio::select! {
            r = stream.read_exact(&mut buf) => {
                match r {
                    Ok(_) => {
                        let len = gout_proto::decode_udp_header(&buf) as usize;
                        if len == 0 { break; }
                        let mut data = vec![0u8; len];
                        if stream.read_exact(&mut data).await.is_err() { break; }
                    }
                    Err(_) => break,
                }
            }
            _ = tokio::signal::ctrl_c() => {
                break;
            }
        }
    }
    println!("UDP 隧道已关闭");
    Ok(())
}

/// 从 server addr 中提取 host（去掉端口部分）
fn server_host(addr: &str) -> &str {
    addr.split(':').next().unwrap_or(addr)
}

use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn parse_tt(s: &str) -> gout_proto::TunnelType {
    gout_proto::TunnelType::parse(s)
}
