/// Gout 配置 — `~/.gout/config.toml`（旧 `~/.goutrc` 自动迁移）。

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

/// gout 数据目录 `~/.gout`
pub fn gout_dir() -> PathBuf {
    let home = if let Ok(h) = std::env::var("HOME") {
        PathBuf::from(h)
    } else {
        dirs::home_dir().expect("cannot find home directory")
    };
    home.join(".gout")
}

/// 配置文件路径（新: `~/.gout/config.toml`，旧: `~/.goutrc`）
pub fn config_path() -> PathBuf {
    let new = gout_dir().join("config.toml");
    if new.exists() {
        return new;
    }
    // 兼容旧路径
    let legacy = gout_dir().with_file_name(".goutrc");
    if legacy.exists() {
        return legacy;
    }
    new // 不存在时返回新路径，下次 write 会创建
}

/// 写入配置文件。写入新位置，自动清理旧文件。
pub fn write(addr: &str, api_key: &str) -> Result<()> {
    let cfg = Config {
        server: ServerConfig {
            addr: addr.to_string(),
            api_key: api_key.to_string(),
        },
    };
    let content = toml::to_string_pretty(&cfg).context("serialize config")?;

    let new_path = gout_dir().join("config.toml");
    std::fs::create_dir_all(gout_dir()).context("create ~/.gout")?;
    std::fs::write(&new_path, &content).context("write ~/.gout/config.toml")?;

    // 删除旧文件
    let legacy = gout_dir().with_file_name(".goutrc");
    if legacy.exists() {
        std::fs::remove_file(&legacy).ok();
    }
    Ok(())
}

/// 读取配置文件。
pub fn read() -> Result<Config> {
    let path = config_path();
    if !path.exists() {
        anyhow::bail!("config not found. Run `gout login <server> <key>` first.\n       looked in: {}", path.display());
    }
    let content = std::fs::read_to_string(&path).context("read config")?;
    let cfg: Config = toml::from_str(&content).context("parse config")?;

    // 如果读的是旧位置，静默迁移到新位置
    let new_path = gout_dir().join("config.toml");
    if path != new_path {
        std::fs::create_dir_all(gout_dir()).ok();
        std::fs::write(&new_path, &content).ok();
        std::fs::remove_file(&path).ok();
    }

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
            // 写入新位置
            assert!(gout_dir().join("config.toml").exists());
        });
    }

    #[test]
    fn test_read_not_found() {
        with_temp_home(|_home| {
            let err = read().unwrap_err();
            assert!(err.to_string().contains("not found"), "got: {err}");
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
    fn test_migrates_legacy_rc() {
        with_temp_home(|home| {
            // 在旧位置写文件
            let legacy = home.join(".goutrc");
            let content = r#"[server]
addr = "old:8080"
api_key = "sk-old"
"#;
            std::fs::write(&legacy, content).unwrap();

            // read 应静默迁移到新位置
            let cfg = read().unwrap();
            assert_eq!(cfg.server.addr, "old:8080");

            assert!(!legacy.exists(), "legacy file should be removed");
            assert!(gout_dir().join("config.toml").exists());
        });
    }

    #[test]
    fn test_config_path_respects_home() {
        with_temp_home(|home| {
            let dir = gout_dir();
            assert!(dir.starts_with(home));
            assert_eq!(dir.file_name().unwrap(), ".gout");
        });
    }
}
