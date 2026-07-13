//! Daemon 进程管理 — 后台隧道生命周期。
//!
//! 管理 `~/.gout/daemon/` 目录下的 PID 文件（`.json`）和日志文件（`.log`）。
//! 所有路径从 [`config::gout_dir`] 派生。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ━━━ 类型 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// PID 文件内容
#[derive(Debug, Serialize, Deserialize)]
pub struct DaemonInfo {
    pub pid: u32,
    pub port: u16,
    pub tunnel_type: String,
    /// 远端服务器主机名（来自 config，如 "frp.freet.tech"）
    #[serde(default)]
    pub server_host: String,
    /// 远端隧道端口（REST API 返回的 public_port）
    #[serde(default)]
    pub public_port: u16,
}

/// 活跃后台隧道条目（list 输出用）
#[derive(Debug)]
pub struct DaemonEntry {
    pub pid: u32,
    pub port: u16,
    pub tunnel_type: String,
    /// 完整远端地址（如 "frp.freet.tech:10000"）
    pub remote: String,
}

// ━━━ Manager ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub struct DaemonManager {
    dir: PathBuf,
}

impl DaemonManager {
    /// 创建一个 DaemonManager，基于 `~/.gout/daemon`。
    pub fn new() -> Self {
        let dir = crate::config::gout_dir().join("daemon");
        Self { dir }
    }

    // ─── 路径 ─────────────────────────────────────────────────

    fn ensure_dir(&self) -> Result<()> {
        std::fs::create_dir_all(&self.dir).context("create daemon dir")
    }

    fn pidfile(&self, port: u16) -> PathBuf {
        self.dir.join(format!("{port}.json"))
    }

    fn logfile(&self, port: u16) -> PathBuf {
        self.dir.join(format!("{port}.log"))
    }

    // ─── 进程检测 ─────────────────────────────────────────────

    #[cfg(unix)]
    fn is_alive(pid: u32) -> bool {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[cfg(not(unix))]
    fn is_alive(pid: u32) -> bool {
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}")])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
            .unwrap_or(false)
    }

    #[cfg(unix)]
    fn terminate(pid: u32) -> Result<()> {
        std::process::Command::new("kill")
            .args([&pid.to_string()])
            .status()
            .context("kill failed")?;
        Ok(())
    }

    #[cfg(not(unix))]
    fn terminate(pid: u32) -> Result<()> {
        std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .status()
            .context("taskkill failed")?;
        Ok(())
    }

    // ─── 公开 API ─────────────────────────────────────────────

    /// 列出活跃隧道（跳过 `.json` 后缀文件以外的条目），清理僵尸记录。
    pub fn list(&self) -> Vec<DaemonEntry> {
        let dir = match std::fs::read_dir(&self.dir) {
            Ok(d) => d,
            Err(_) => return vec![],
        };

        let mut entries = Vec::new();
        let mut stale = Vec::new();

        for entry in dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let info: DaemonInfo = match serde_json::from_str(&content) {
                Ok(i) => i,
                Err(_) => continue,
            };

            if Self::is_alive(info.pid) {
                let remote = if info.server_host.is_empty() || info.public_port == 0 {
                    String::new()
                } else {
                    format!("{}:{}", info.server_host, info.public_port)
                };
                entries.push(DaemonEntry {
                    pid: info.pid,
                    port: info.port,
                    tunnel_type: info.tunnel_type,
                    remote,
                });
            } else {
                stale.push(path);
            }
        }

        // 清理僵尸
        for p in &stale {
            std::fs::remove_file(p).ok();
            if let Some(port) = p.file_stem().and_then(|s| s.to_str()) {
                if let Ok(port_num) = port.parse::<u16>() {
                    std::fs::remove_file(self.logfile(port_num)).ok();
                }
            }
        }

        entries
    }

    /// 启动后台隧道（父进程已创建隧道，传入 token + data_port）。
    pub fn start_with_tunnel(
        &self,
        tunnel_type: &str,
        port: u16,
        token: u64,
        data_port: u16,
        public_port: u16,
        server_host: &str,
    ) -> Result<u32> {
        self.ensure_dir()?;
        let pidfile = self.pidfile(port);

        Self::check_existing(&pidfile, port)?;

        let exe = std::env::current_exe().context("cannot get own exe path")?;
        let logfile = self.logfile(port);
        let log_handle = std::fs::File::create(&logfile).context("create log file")?;

        let child = std::process::Command::new(&exe)
            .args([tunnel_type, &port.to_string()])
            .env("GOUT_DAEMON_PIDFILE", pidfile.to_str().unwrap())
            .env("GOUT_DAEMON_TOKEN", token.to_string())
            .env("GOUT_DAEMON_DATA_PORT", data_port.to_string())
            .stdin(std::process::Stdio::null())
            .stdout(log_handle.try_clone().context("clone log handle")?)
            .stderr(log_handle)
            .spawn()
            .context("failed to spawn daemon")?;

        let info = DaemonInfo {
            pid: child.id(),
            port,
            tunnel_type: tunnel_type.to_string(),
            server_host: server_host.to_string(),
            public_port,
        };
        std::fs::write(&pidfile, serde_json::to_string_pretty(&info)?)?;

        Ok(child.id())
    }

    /// 停止后台隧道。
    pub fn kill(&self, port: u16) -> Result<()> {
        let pidfile = self.pidfile(port);

        if !pidfile.exists() {
            anyhow::bail!("no daemon record found for port {port}");
        }

        let content = std::fs::read_to_string(&pidfile)?;
        let info: DaemonInfo = serde_json::from_str(&content)?;

        if !Self::is_alive(info.pid) {
            println!("[!] tunnel on port {} (PID {}) already exited", port, info.pid);
            Self::cleanup(&pidfile, &self.logfile(port));
            return Ok(());
        }

        Self::terminate(info.pid)?;
        Self::cleanup(&pidfile, &self.logfile(port));
        println!("[+] tunnel on port {} (PID {}) stopped", port, info.pid);
        Ok(())
    }

    /// 读取日志文件内容。
    pub fn read_log(&self, port: u16) -> Result<String> {
        let logfile = self.logfile(port);
        if !logfile.exists() {
            anyhow::bail!("no log file found for port {port}");
        }
        std::fs::read_to_string(&logfile).context("read log")
    }

    /// 跟踪日志（Unix: tail -f，Windows: 一次输出）。
    pub fn follow_log(&self, port: u16) -> Result<()> {
        let logfile = self.logfile(port);
        if !logfile.exists() {
            anyhow::bail!("no log file found for port {port}");
        }

        #[cfg(unix)]
        {
            let status = std::process::Command::new("tail")
                .args(["-f", &logfile.to_string_lossy()])
                .status()
                .context("tail failed")?;
            if !status.success() {
                anyhow::bail!("tail exited with {}", status);
            }
            Ok(())
        }

        #[cfg(not(unix))]
        {
            let content = std::fs::read_to_string(&logfile)?;
            print!("{content}");
            println!("[!] follow mode (-f) not supported on Windows");
            Ok(())
        }
    }

    // ─── 内部 ─────────────────────────────────────────────────

    fn check_existing(pidfile: &Path, port: u16) -> Result<()> {
        if pidfile.exists() {
            let content = std::fs::read_to_string(pidfile)?;
            if let Ok(info) = serde_json::from_str::<DaemonInfo>(&content) {
                if Self::is_alive(info.pid) {
                    anyhow::bail!(
                        "tunnel on port {} already running (PID {})",
                        port, info.pid
                    );
                }
            }
            // 僵尸，清扫
            std::fs::remove_file(pidfile)?;
            // 也清除对应日志文件
            let logfile = pidfile.with_extension("log");
            std::fs::remove_file(logfile).ok();
        }
        Ok(())
    }

    fn cleanup(pidfile: &Path, logfile: &Path) {
        std::fs::remove_file(pidfile).ok();
        std::fs::remove_file(logfile).ok();
    }
}

impl Default for DaemonManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl DaemonManager {
    /// 测试用：指定目录构造，避免全局 HOME 变量竞争。
    pub fn test_new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

// ━━━ 测试 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    fn with_temp_mgr(f: impl FnOnce(&DaemonManager)) {
        let tmp = tempfile::tempdir().expect("temp dir");
        let dir = tmp.path().join("daemon");
        let mgr = DaemonManager::test_new(dir);
        f(&mgr);
    }

    fn write_pid(mgr: &DaemonManager, port: u16, pid: u32) {
        let info = DaemonInfo {
            pid,
            port,
            tunnel_type: "tcp".into(),
            server_host: String::new(),
            public_port: 0,
        };
        std::fs::create_dir_all(&mgr.dir).unwrap();
        std::fs::write(
            mgr.pidfile(port),
            serde_json::to_string_pretty(&info).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn list_empty_when_no_dir() {
        let mgr = DaemonManager::test_new(PathBuf::from("/nonexistent/gout/daemon"));
        assert!(mgr.list().is_empty());
    }

    #[test]
    fn list_returns_pid_files() {
        with_temp_mgr(|mgr| {
            write_pid(mgr, 4000, 999_999_999);
            let entries = mgr.list();
            assert!(entries.is_empty(), "stale PID should be cleaned up");
            assert!(!mgr.pidfile(4000).exists(), "PID file should be removed");
        });
    }

    #[test]
    fn list_shows_remote() {
        with_temp_mgr(|mgr| {
            let info = DaemonInfo {
                pid: 999_999_999,
                port: 4000,
                tunnel_type: "http".into(),
                server_host: "example.com".into(),
                public_port: 10001,
            };
            std::fs::create_dir_all(&mgr.dir).unwrap();
            std::fs::write(
                mgr.pidfile(4000),
                serde_json::to_string_pretty(&info).unwrap(),
            )
            .unwrap();
            let _entries = mgr.list();
        });
    }

    #[test]
    fn kill_missing_port_errors() {
        with_temp_mgr(|mgr| {
            let err = mgr.kill(9999).unwrap_err();
            assert!(err.to_string().contains("no daemon record"));
        });
    }

    #[test]
    fn read_log_nonexistent_port() {
        with_temp_mgr(|mgr| {
            let err = mgr.read_log(9999).unwrap_err();
            assert!(err.to_string().contains("no log file"));
        });
    }

    #[test]
    fn pidfile_and_logfile_paths() {
        with_temp_mgr(|mgr| {
            let pf = mgr.pidfile(4000);
            assert!(pf.to_string_lossy().ends_with("4000.json"));
            let lf = mgr.logfile(4000);
            assert!(lf.to_string_lossy().ends_with("4000.log"));
        });
    }
}
