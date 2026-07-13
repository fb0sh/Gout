mod cli;
mod config;
mod tunnel;

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

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
            println!("{:<20}  {:>5}  {:>4}  {:>7}", "TUNNEL_ID", "PORT", "TYPE", "STATUS");
            for t in &tunnels {
                let status = if t.connected { "active" } else { "waiting" };
                println!("{:<20}  {:>5}  {:>4}  {:>7}", t.token.to_string(), t.public_port, t.tunnel_type, status);
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

// ━━━ Daemon 管理 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Serialize, Deserialize)]
struct DaemonInfo {
    pid: u32,
    port: u16,
    tunnel_type: String,
}

fn daemon_dir() -> PathBuf {
    let home = dirs::home_dir().expect("cannot find home directory");
    home.join(".gout").join("daemon")
}

fn daemon_pidfile(port: u16) -> PathBuf {
    daemon_dir().join(format!("{port}.json"))
}

/// 检查进程是否存活
#[cfg(unix)]
fn is_process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_process_alive(pid: u32) -> bool {
    std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}")])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
        .unwrap_or(false)
}

/// 停止进程
#[cfg(unix)]
fn kill_process(pid: u32) -> Result<()> {
    std::process::Command::new("kill")
        .args([&pid.to_string()])
        .status()
        .context("kill failed")?;
    Ok(())
}

#[cfg(not(unix))]
fn kill_process(pid: u32) -> Result<()> {
    std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .status()
        .context("taskkill failed")?;
    Ok(())
}

/// 处理 `tcp/udp/http -d` 命令：在后台启动隧道
fn cmd_start_daemon(tunnel_type: &str, port: u16) -> Result<()> {
    let exe = std::env::current_exe().context("cannot get own exe path")?;
    let dir = daemon_dir();
    std::fs::create_dir_all(&dir).context("create daemon dir")?;
    let pidfile = daemon_pidfile(port);

    // 检查是否已在运行
    if pidfile.exists() {
        let content = std::fs::read_to_string(&pidfile)?;
        if let Ok(info) = serde_json::from_str::<DaemonInfo>(&content) {
            if is_process_alive(info.pid) {
                anyhow::bail!(
                    "tunnel on port {} already running (PID {})",
                    port, info.pid
                );
            }
        }
        println!("[!] removing stale daemon record for port {}", port);
        std::fs::remove_file(&pidfile)?;
    }

    // 启动子进程（不带 -d，静默运行）
    let child = std::process::Command::new(&exe)
        .args([tunnel_type, &port.to_string()])
        .env("GOUT_DAEMON_PIDFILE", pidfile.to_str().unwrap())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("failed to spawn daemon")?;

    let info = DaemonInfo {
        pid: child.id(),
        port,
        tunnel_type: tunnel_type.to_string(),
    };
    std::fs::write(&pidfile, serde_json::to_string_pretty(&info)?)?;

    println!("[+] tunnel started in background (PID: {})", child.id());
    println!("    `gout list` to check status");
    println!("    `gout kill {port}` to stop");
    Ok(())
}

/// 处理 `kill` 命令：停止后台隧道
fn cmd_kill(port: u16) -> Result<()> {
    let pidfile = daemon_pidfile(port);

    if !pidfile.exists() {
        anyhow::bail!("no daemon record found for port {port}");
    }

    let content = std::fs::read_to_string(&pidfile)?;
    let info: DaemonInfo = serde_json::from_str(&content)?;

    if !is_process_alive(info.pid) {
        println!("[!] tunnel on port {} (PID {}) already exited", port, info.pid);
        std::fs::remove_file(&pidfile)?;
        return Ok(());
    }

    kill_process(info.pid)?;
    std::fs::remove_file(&pidfile)?;
    println!("[-] tunnel on port {} (PID {}) stopped", port, info.pid);
    Ok(())
}
