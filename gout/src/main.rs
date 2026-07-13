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

// ━━━ List（按 server 分组展示） ━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn cmd_list() -> Result<()> {
    let servers = config::list_servers().unwrap_or_default();
    let mgr = daemon::DaemonManager::new();
    let entries = mgr.list();

    if servers.is_empty() && entries.is_empty() {
        println!("[*] no servers configured and no active tunnels");
        return Ok(());
    }

    for (name, sc, _is_default) in &servers {
        let host = sc.addr.split(':').next().unwrap_or(&sc.addr);
        let mut sv_entries: Vec<_> = entries
            .iter()
            .filter(|e| e.remote.starts_with(host))
            .collect();
        sv_entries.sort_by_key(|e| e.port);

        println!("{name}  ({})", sc.addr);
        if sv_entries.is_empty() {
            println!("  (no active tunnels)\n");
        } else {
            println!("  {:>5}  {:<26}  {:>4}  {:>6}  {:>8}", "PORT", "REMOTE", "TYPE", "PID", "STATUS");
            for e in &sv_entries {
                let remote: &str = if e.remote.is_empty() { "-" } else { &e.remote };
                println!("  {:>5}  {:<26}  {:>4}  {:>6}  {:>8}", e.port, remote, e.tunnel_type, e.pid, "alive");
            }
            println!();
        }
    }

    // 未匹配到任何 server 的孤立隧道
    let unmatched: Vec<_> = entries
        .iter()
        .filter(|e| !servers.iter().any(|(_, sc, _)| {
            let host = sc.addr.split(':').next().unwrap_or(&sc.addr);
            e.remote.starts_with(host)
        }))
        .collect();
    if !unmatched.is_empty() {
        println!("(other)");
        println!("  {:>5}  {:<26}  {:>4}  {:>6}  {:>8}", "PORT", "REMOTE", "TYPE", "PID", "STATUS");
        for e in &unmatched {
            let remote: &str = if e.remote.is_empty() { "-" } else { &e.remote };
            println!("  {:>5}  {:<26}  {:>4}  {:>6}  {:>8}", e.port, remote, e.tunnel_type, e.pid, "alive");
        }
        println!();
    }

    Ok(())
}

// ━━━ Show（server + tunnel 概览） ━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn cmd_show() -> Result<()> {
    cmd_list()?;
    let servers = config::list_servers().unwrap_or_default();
    if !servers.is_empty() {
        println!("(use `gout default <name>` to change default)");
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
