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
    println!("[+] saved to ~/.goutrc (server: {server})");
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
            println!("[*] no active tunnels");
        } else {
            println!("{:>8}  {:>5}  {:>4}  {}", "TOKEN", "PORT", "TYPE", "STATUS");
            for t in &tunnels {
                let status = if t.has_signal { "active" } else { "waiting" };
                let token_short = &t.token.to_string()[..8.min(t.token.to_string().len())];
                println!("{:>8}  {:>5}  {:>4}  {}", token_short, t.public_port, t.tunnel_type, status);
            }
        }
        Ok(())
    })
}

/// 处理 `tcp/udp/http` 命令
fn cmd_tunnel(tunnel_type: &str, local_port: u16) -> Result<()> {
    let cfg = config::read()?;
    let tt = gout_api::TunnelType::parse(tunnel_type);

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        tunnel::TunnelSession::create(cfg, tt, local_port).await?;
        Ok(())
    })
}
