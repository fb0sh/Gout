mod cli;
mod config;
mod daemon;
mod tunnel;

use anyhow::Result;

fn main() -> Result<()> {
    tracing_subscriber::fmt().init();
    cli::Cli::run()
}

/// 处理 `login` 命令
fn cmd_login(server: &str, key: &str) -> Result<()> {
    config::write(server, key)?;
    println!("[+] saved to ~/.gout/config.toml (server: {server})");
    Ok(())
}

/// 处理 `list` 命令
fn cmd_list() -> Result<()> {
    let mgr = daemon::DaemonManager::new();
    let entries = mgr.list();
    if entries.is_empty() {
        println!("[*] no active tunnels");
        return Ok(());
    }
    println!("{:>5}  {:>4}  {:>6}  {:>8}", "PORT", "TYPE", "PID", "STATUS");
    for e in &entries {
        println!("{:>5}  {:>4}  {:>6}  {:>8}", e.port, e.tunnel_type, e.pid, "alive");
    }
    Ok(())
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

/// 处理 `tcp/udp/http -d` 命令
fn cmd_start_daemon(tunnel_type: &str, port: u16) -> Result<()> {
    let mgr = daemon::DaemonManager::new();
    let pid = mgr.start(tunnel_type, port)?;
    println!("[+] tunnel started in background (PID: {pid})");
    println!("    `gout list` to check status");
    println!("    `gout log {port}` to view logs");
    println!("    `gout kill {port}` to stop");
    Ok(())
}

/// 处理 `log` 命令
fn cmd_log(port: u16, follow: bool) -> Result<()> {
    let mgr = daemon::DaemonManager::new();
    if follow {
        mgr.follow_log(port)
    } else {
        let content = mgr.read_log(port)?;
        print!("{content}");
        Ok(())
    }
}

/// 处理 `kill` 命令
fn cmd_kill(port: u16) -> Result<()> {
    daemon::DaemonManager::new().kill(port)
}
