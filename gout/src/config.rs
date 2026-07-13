/// Gout 配置 — `~/.gout/config.toml`（多 server 支持）。
///
/// 旧 `~/.goutrc` / 旧单 server 格式自动迁移。

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ━━━ 新版多 server 配置 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    #[serde(default = "default_server_name")]
    pub default_server: String,
    pub servers: HashMap<String, ServerConfig>,
}

fn default_server_name() -> String {
    "default".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerConfig {
    pub addr: String,
    pub api_key: String,
}

// ━━━ 旧版单 server 格式（用于迁移） ━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Deserialize)]
struct OldConfig {
    server: OldServerConfig,
}

#[derive(Debug, Deserialize)]
struct OldServerConfig {
    addr: String,
    api_key: String,
}

// ━━━ 路径 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub fn gout_dir() -> PathBuf {
    let home = if let Ok(h) = std::env::var("HOME") {
        PathBuf::from(h)
    } else {
        dirs::home_dir().expect("cannot find home directory")
    };
    home.join(".gout")
}

fn config_path() -> PathBuf {
    let new = gout_dir().join("config.toml");
    if new.exists() {
        return new;
    }
    let legacy = gout_dir().with_file_name(".goutrc");
    if legacy.exists() {
        return legacy;
    }
    new
}

// ━━━ 公开 API ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 解析配置文件，返回新版 Config。旧格式自动转换。旧 `.goutrc` 静默迁移。
pub fn read() -> Result<Config> {
    let path = config_path();
    if !path.exists() {
        anyhow::bail!(
            "config not found. Run `gout login <server> <key>` first.\n       looked in: {}",
            path.display()
        );
    }
    let content = std::fs::read_to_string(&path).context("read config")?;

    // 尝试解析新版；失败则尝试旧版迁移
    let cfg = match toml::from_str::<Config>(&content) {
        Ok(c) => c,
        Err(_) => {
            let old: OldConfig =
                toml::from_str(&content).context("config format unrecognized")?;
            let mut servers = HashMap::new();
            servers.insert(
                "default".to_string(),
                ServerConfig {
                    addr: old.server.addr,
                    api_key: old.server.api_key,
                },
            );
            let cfg = Config {
                default_server: "default".to_string(),
                servers,
            };
            // 写回新版格式
            let new_content = toml::to_string_pretty(&cfg)?;
            let new_path = gout_dir().join("config.toml");
            std::fs::create_dir_all(gout_dir()).ok();
            std::fs::write(&new_path, &new_content).ok();
            // 如果是旧版位置，清除旧文件
            if path != new_path {
                std::fs::remove_file(&path).ok();
            }
            cfg
        }
    };

    Ok(cfg)
}

/// 写入一个 server 到配置。如果 key 为空表示只保存 server 信息。
pub fn write(name: &str, addr: &str, api_key: &str) -> Result<()> {
    let mut cfg = if config_path().exists() {
        read().unwrap_or_else(|_| Config {
            default_server: default_server_name(),
            servers: HashMap::new(),
        })
    } else {
        Config {
            default_server: default_server_name(),
            servers: HashMap::new(),
        }
    };

    cfg.servers.insert(
        name.to_string(),
        ServerConfig {
            addr: addr.to_string(),
            api_key: api_key.to_string(),
        },
    );

    // 第一次添加的 server 自动设为默认
    if cfg.servers.len() == 1 {
        cfg.default_server = name.to_string();
    }

    let content = toml::to_string_pretty(&cfg).context("serialize config")?;
    let new_path = gout_dir().join("config.toml");
    std::fs::create_dir_all(gout_dir()).context("create ~/.gout")?;
    std::fs::write(&new_path, &content).context("write config")?;

    // 清理旧文件
    let legacy = gout_dir().with_file_name(".goutrc");
    if legacy.exists() {
        std::fs::remove_file(&legacy).ok();
    }
    Ok(())
}

/// 根据 server 名解析出 ServerConfig。空名或 "default" 返回默认 server。
pub fn resolve(name: Option<&str>) -> Result<ServerConfig> {
    let cfg = read()?;
    let key = name.unwrap_or(&cfg.default_server);
    let key = if key.is_empty() { &cfg.default_server } else { key };
    cfg.servers
        .get(key)
        .cloned()
        .or_else(|| {
            // 也支持直接传 addr 匹配
            cfg.servers.values().find(|s| s.addr == key).cloned()
        })
        .context(format!("server {key:?} not found"))
}

/// 列出所有 server 名及默认标记
pub fn list_servers() -> Result<Vec<(String, ServerConfig, bool)>> {
    let cfg = read()?;
    let mut list: Vec<_> = cfg
        .servers
        .into_iter()
        .map(|(name, sc)| {
            let is_default = name == cfg.default_server;
            (name, sc, is_default)
        })
        .collect();
    list.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0))); // 默认排最前
    Ok(list)
}

// ━━━ 测试 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static HOME_LOCK: Mutex<()> = Mutex::new(());

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
            write("default", "example.com:8080", "sk-test123").unwrap();
            let cfg = read().unwrap();
            let s = cfg.servers.get("default").unwrap();
            assert_eq!(s.addr, "example.com:8080");
            assert_eq!(s.api_key, "sk-test123");
            assert_eq!(cfg.default_server, "default");
        });
    }

    #[test]
    fn test_multi_server() {
        with_temp_home(|_home| {
            write("prod", "prod.com:8080", "sk-prod").unwrap();
            write("dev", "dev.com:8080", "sk-dev").unwrap();
            let cfg = read().unwrap();
            assert_eq!(cfg.servers.len(), 2);
            assert_eq!(cfg.default_server, "prod"); // 第一个自动默认
        });
    }

    #[test]
    fn test_resolve_default() {
        with_temp_home(|_home| {
            write("main", "m:8080", "sk-m").unwrap();
            let s = resolve(None).unwrap();
            assert_eq!(s.addr, "m:8080");
        });
    }

    #[test]
    fn test_resolve_by_addr() {
        with_temp_home(|_home| {
            write("x", "x.com:8080", "sk-x").unwrap();
            let s = resolve(Some("x.com:8080")).unwrap();
            assert_eq!(s.api_key, "sk-x");
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
    fn test_migrates_old_format() {
        with_temp_home(|home| {
            let old_content = r#"[server]
addr = "old:8080"
api_key = "sk-old"
"#;
            std::fs::write(home.join(".goutrc"), old_content).unwrap();

            let cfg = read().unwrap();
            let s = cfg.servers.get("default").unwrap();
            assert_eq!(s.addr, "old:8080");
            assert_eq!(s.api_key, "sk-old");
            assert_eq!(cfg.default_server, "default");

            // 旧文件已迁移
            assert!(!home.join(".goutrc").exists());
            assert!(config_path().exists());
        });
    }

    #[test]
    fn test_list_servers() {
        with_temp_home(|_home| {
            write("b", "b:1", "k").unwrap();
            write("a", "a:2", "k").unwrap();
            let list = list_servers().unwrap();
            assert!(list.len() >= 2);
            // 第一条是默认（b，先添加的）
            assert_eq!(list[0].0, "b");
            assert!(list[0].2); // is_default
        });
    }
}
