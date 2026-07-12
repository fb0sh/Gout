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
