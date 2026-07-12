mod cli;
mod config;

use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn main() -> Result<()> {
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
    let tt = gout_api::TunnelType::parse(tunnel_type);
    println!("🔗 创建 {tunnel_type} 隧道 {local_port} → {}", cfg.server.addr);

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        // 1. 用 GoutClient 创建隧道
        let gout = gout_api::client::GoutClient::new(&cfg.server.addr, &cfg.server.api_key);
        let tunnel = gout.create_tunnel(tt, local_port).await?;

        println!("✅ 隧道已创建");
        println!("   公网端口: {}  →  localhost:{}", tunnel.public_port, local_port);
        println!("   数据端口: {}", tunnel.data_port);
        println!("   按 Ctrl+C 关闭隧道");

        // 2. 连接数据端口 + 握手（信号通道）
        let data_addr = format!("{}:{}", server_host(&cfg.server.addr), tunnel.data_port);
        let mut stream = tokio::net::TcpStream::connect(&data_addr)
            .await
            .context("connect to data port failed")?;

        let handshake = gout_api::encode_handshake(tunnel.token, tt);
        stream.write_all(&handshake).await.context("send handshake failed")?;

        let mut status = [0u8; 1];
        stream.read_exact(&mut status).await?;
        if status[0] != gout_api::STATUS_OK {
            anyhow::bail!("handshake rejected by server");
        }

        println!("   信号通道已建立，等待外部连接...");
        println!("   隧道已就绪！");

        // 3. 信号通道循环 + 数据转发
        if tunnel_type == "udp" {
            run_udp_channel(stream, &gout, tunnel.token).await?;
        } else {
            run_tcp_signal_channel(stream, &gout, tunnel.token, local_port).await?;
        }

        // 4. 清理
        gout.delete_tunnel(tunnel.token).await.ok();
        println!("隧道已关闭");
        Ok(())
    })
}

/// TCP 信号通道循环
async fn run_tcp_signal_channel(
    mut stream: tokio::net::TcpStream,
    gout: &gout_api::client::GoutClient,
    token: u64,
    local_port: u16,
) -> Result<()> {
    let mut buf = [0u8; 1];
    loop {
        tokio::select! {
            r = stream.read(&mut buf) => {
                match r {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if buf[0] == gout_api::SIGNAL_NEW_CONN {
                            let gout = gout_api::client::GoutClient::new(gout.server_addr(), gout.api_key());
                            tokio::spawn(async move {
                                handle_data_channel(gout, token, local_port).await;
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
    gout: gout_api::client::GoutClient,
    token: u64,
    local_port: u16,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let data_addr = format!("{}:8081", gout.server_addr());
    let mut stream = match tokio::net::TcpStream::connect(&data_addr).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("connect data port failed: {e}");
            return;
        }
    };

    let handshake = gout_api::encode_handshake(token, gout_api::TunnelType::Tcp);
    if stream.write_all(&handshake).await.is_err() { return; }

    let mut status = [0u8; 1];
    if stream.read_exact(&mut status).await.is_err() || status[0] != gout_api::STATUS_OK {
        eprintln!("data channel handshake rejected");
        return;
    }

    let mut local = match tokio::net::TcpStream::connect(format!("127.0.0.1:{local_port}")).await {
        Ok(s) => s,
        Err(_) => {
            let _ = stream.write_all(&[0u8; 1]).await;
            eprintln!("连接 localhost:{local_port} 失败 — 本地服务未启动？");
            return;
        }
    };

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
    _gout: &gout_api::client::GoutClient,
    _token: u64,
) -> Result<()> {
    let mut buf = [0u8; gout_api::UDP_FRAME_HEADER];
    loop {
        tokio::select! {
            r = stream.read_exact(&mut buf) => {
                match r {
                    Ok(_) => {
                        let len = gout_api::decode_udp_header(&buf) as usize;
                        if len == 0 { break; }
                        let mut data = vec![0u8; len];
                        if stream.read_exact(&mut data).await.is_err() { break; }
                    }
                    Err(_) => break,
                }
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    Ok(())
}

/// 从 server addr 中提取 host
fn server_host(addr: &str) -> &str {
    addr.split(':').next().unwrap_or(addr)
}

fn parse_tt(s: &str) -> gout_api::TunnelType {
    gout_api::TunnelType::parse(s)
}
