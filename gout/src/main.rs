mod cli;
mod config;
mod tunnel;

use anyhow::Result;

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

/// 处理 `list` 命令
fn cmd_list() -> Result<()> {
    let cfg = config::read()?;
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let gout = gout_api::client::GoutClient::new(&cfg.server.addr, &cfg.server.api_key);
        let tunnels = gout.list_tunnels().await?;
        if tunnels.is_empty() {
            println!("No active tunnels");
        } else {
            println!("{:<8} {:<6} {:<12} {:<6} {}", "TOKEN", "TYPE", "PUBLIC", "KEY", "STATUS");
            for t in &tunnels {
                let status = if t.has_signal { "active" } else { "waiting" };
                println!(
                    "{:<8} {:<6} {:<12} {:<6} {}",
                    &t.token.to_string()[..8.min(t.token.to_string().len())],
                    t.tunnel_type,
                    t.public_port,
                    t.key_name,
                    status,
                );
            }
        }
        Ok(())
    })
}

/// 处理 `tcp/udp/http` 命令
fn cmd_tunnel(tunnel_type: &str, local_port: u16) -> Result<()> {
    let cfg = config::read()?;
    let tt = gout_api::TunnelType::parse(tunnel_type);
    println!("🔗 创建 {tunnel_type} 隧道 {local_port} → {}", cfg.server.addr);

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        tunnel::TunnelSession::create(cfg, tt, local_port).await?;
        Ok(())
    })
}
