/// ~/.goutrc 读写

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerConfig {
    pub addr: String,
    pub api_key: String,
}

/// 配置文件路径 ~/.goutrc
pub fn config_path() -> PathBuf {
    // 优先读取 HOME 环境变量（测试用），否则用 dirs crate
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".goutrc")
    } else {
        let home = dirs::home_dir().expect("cannot find home directory");
        home.join(".goutrc")
    }
}

/// 写入配置文件
pub fn write(addr: &str, api_key: &str) -> Result<()> {
    let cfg = Config {
        server: ServerConfig {
            addr: addr.to_string(),
            api_key: api_key.to_string(),
        },
    };
    let content = toml::to_string_pretty(&cfg).context("serialize config")?;
    std::fs::write(config_path(), content).context("write ~/.goutrc")?;
    Ok(())
}

/// 读取配置文件，None 表示文件不存在
pub fn read() -> Result<Config> {
    let path = config_path();
    if !path.exists() {
        anyhow::bail!("~/.goutrc not found. Run `gout login <server> <key>` first.");
    }
    let content = std::fs::read_to_string(&path).context("read ~/.goutrc")?;
    let cfg: Config = toml::from_str(&content).context("parse ~/.goutrc")?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static HOME_LOCK: Mutex<()> = Mutex::new(());

    /// 创建一个临时 HOME 目录并设置 HOME 变量
    fn with_temp_home(f: impl FnOnce(&std::path::Path)) {
        let _guard = HOME_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().expect("temp dir");
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let old = std::env::var_os("HOME");
        std::env::set_var("HOME", &home);
        f(&home);
        match old {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn test_write_then_read() {
        with_temp_home(|_home| {
            write("example.com:8080", "sk-test123").unwrap();
            let cfg = read().unwrap();
            assert_eq!(cfg.server.addr, "example.com:8080");
            assert_eq!(cfg.server.api_key, "sk-test123");
        });
    }

    #[test]
    fn test_read_not_found() {
        with_temp_home(|_home| {
            let err = read().unwrap_err();
            assert!(err.to_string().contains("not found"));
        });
    }

    #[test]
    fn test_write_overwrites() {
        with_temp_home(|_home| {
            write("a:1", "key-a").unwrap();
            write("b:2", "key-b").unwrap();
            let cfg = read().unwrap();
            assert_eq!(cfg.server.addr, "b:2");
            assert_eq!(cfg.server.api_key, "key-b");
        });
    }

    #[test]
    fn test_config_path_respects_home() {
        with_temp_home(|home| {
            let path = config_path();
            assert!(path.starts_with(home));
            assert_eq!(path.file_name().unwrap(), ".goutrc");
        });
    }
}
