mod cli;
mod config;
mod daemon;
mod tunnel;

use std::path::PathBuf;

use anyhow::{Context, Result};

fn main() -> Result<()> {
    tracing_subscriber::fmt().init();
    cli::Cli::run()
}

// ━━━ Login ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn cmd_login(name: &str, server: &str, key: &str) -> Result<()> {
    config::write(name, server, key)?;
    println!("[+] saved server {name:?} ({server}) to ~/.gout/config.toml");
    Ok(())
}

// ━━━ List（本地 daemon） ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn cmd_list() -> Result<()> {
    let mgr = daemon::DaemonManager::new();
    let entries = mgr.list();
    if entries.is_empty() {
        println!("[*] no active tunnels");
        return Ok(());
    }
    println!("{:>5}  {:<26}  {:>4}  {:>6}  {:>8}", "PORT", "REMOTE", "TYPE", "PID", "STATUS");
    for e in &entries {
        let remote: &str = if e.remote.is_empty() { "-" } else { &e.remote };
        println!("{:>5}  {:<26}  {:>4}  {:>6}  {:>8}", e.port, remote, e.tunnel_type, e.pid, "alive");
    }
    Ok(())
}

// ━━━ Show（server + tunnel 概览） ━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn cmd_show() -> Result<()> {
    // 服务器列表
    let servers = config::list_servers().unwrap_or_default();
    if servers.is_empty() {
        println!("[*] no servers configured");
        println!("    run `gout login <name> <addr> <key>` to add one");
    } else {
        println!("Servers:");
        for (name, sc, is_default) in &servers {
            let def = if *is_default { " ← default" } else { "" };
            println!("  {name}{def}");
            println!("    addr:     {}", sc.addr);
            println!("    api_key:  …{}", &sc.api_key[sc.api_key.len().saturating_sub(8)..]);
        }
        println!("  (use `gout default <name>` to change)");
    }

    // 本地后台隧道
    let mgr = daemon::DaemonManager::new();
    let entries = mgr.list();
    if entries.is_empty() {
        println!("\n[*] no active tunnels");
    } else {
        println!("\nActive tunnels:");
        println!("{:>5}  {:<26}  {:>4}  {:>6}  {:>8}", "PORT", "REMOTE", "TYPE", "PID", "STATUS");
        for e in &entries {
            let remote: &str = if e.remote.is_empty() { "-" } else { &e.remote };
            println!("{:>5}  {:<26}  {:>4}  {:>6}  {:>8}", e.port, remote, e.tunnel_type, e.pid, "alive");
        }
    }

    Ok(())
}

// ━━━ Tunnel 命令 / 辅助 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 解析 server 参数，返回 ServerConfig
fn resolve_server(server: Option<&str>) -> Result<config::ServerConfig> {
    config::resolve(server).context(
        "no server configured. Run `gout login <name> <addr> <key>` first.",
    )
}

fn cmd_tunnel(tunnel_type: &str, local_port: u16, server: Option<&str>) -> Result<()> {
    let sc = resolve_server(server)?;
    let tt = gout_api::TunnelType::parse(tunnel_type);

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        tunnel::TunnelSession::create(sc, tt, local_port).await?;
        Ok(())
    })
}

fn cmd_start_daemon(tunnel_type: &str, port: u16, server: Option<&str>) -> Result<()> {
    let sc = resolve_server(server)?;
    let tt = gout_api::TunnelType::parse(tunnel_type);
    let server_host = sc.addr.split(':').next().unwrap_or(&sc.addr);

    // 父进程先通过 REST API 创建隧道
    let rt = tokio::runtime::Runtime::new()?;
    let (token, data_port, public_port) = rt.block_on(async {
        let gout = gout_api::client::GoutClient::new(&sc.addr, &sc.api_key);
        let tun = gout.create_tunnel(tt, port).await?;
        anyhow::Ok((tun.token, tun.data_port, tun.public_port))
    })?;

    // 显示映射
    let local_url = if tt == gout_api::TunnelType::Http {
        format!("http://127.0.0.1:{port}")
    } else {
        format!("127.0.0.1:{port}")
    };
    let remote_url = if tt == gout_api::TunnelType::Http {
        format!("http://{server_host}:{public_port}")
    } else {
        format!("{server_host}:{public_port}")
    };
    println!("[+] {} tunnel: {} -> {}", tunnel_type, local_url, remote_url);

    // 启动子进程
    let mgr = daemon::DaemonManager::new();
    let pid = mgr.start_with_tunnel(tunnel_type, port, token, data_port, public_port, server_host)?;
    println!("[+] tunnel started in background (PID: {pid})");
    println!("    `gout list` to check status");
    println!("    `gout log {port}` to view logs");
    println!("    `gout kill {port}` to stop");
    Ok(())
}

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

fn cmd_kill(port: u16) -> Result<()> {
    daemon::DaemonManager::new().kill(port)
}

fn cmd_set_default(name: &str) -> Result<()> {
    config::set_default(name)?;
    println!("[+] default server set to {name:?}");
    Ok(())
}
