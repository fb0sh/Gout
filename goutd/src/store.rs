/// Key 持久化存储 — TOML 文件读写。
///
/// 两种 key：
/// - admin: `admin-api-key`，用于管理面板和 key 管理
/// - tunnel: `api-key`，用于客户端创建隧道

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyEntry {
    pub key: String,
    pub name: String,
    pub created_at: String,
    #[serde(default)]
    pub admin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct KeysFile {
    #[serde(default)]
    keys: Vec<KeyEntry>,
}

pub struct KeyStore {
    path: PathBuf,
    mu: Mutex<()>,
}

impl KeyStore {
    pub fn new(data_dir: &Path) -> Self {
        std::fs::create_dir_all(data_dir).ok();
        Self {
            path: data_dir.join("keys.toml"),
            mu: Mutex::new(()),
        }
    }

    /// 加载所有 key
    pub async fn load(&self) -> Result<Vec<KeyEntry>> {
        let _lock = self.mu.lock().await;
        if !self.path.exists() {
            return Ok(vec![]);
        }
        let content = tokio::fs::read_to_string(&self.path).await.context("read keys.toml")?;
        let keys: KeysFile = toml::from_str(&content).context("parse keys.toml")?;
        Ok(keys.keys)
    }

    /// 添加一个 key 并持久化。
    /// 如果 `admin: true` 且已存在 admin key，则返回错误。
    pub async fn add(&self, entry: KeyEntry) -> Result<()> {
        let _lock = self.mu.lock().await;
        let mut keys = if self.path.exists() {
            let content = tokio::fs::read_to_string(&self.path).await?;
            toml::from_str::<KeysFile>(&content)?
        } else {
            KeysFile::default()
        };

        // 只允许一个 admin key
        if entry.admin && keys.keys.iter().any(|k| k.admin) {
            anyhow::bail!("an admin key already exists");
        }

        keys.keys.push(entry);
        self.write(&keys).await
    }

    /// 删除一个 key 并持久化
    pub async fn delete(&self, key: &str) -> Result<bool> {
        let _lock = self.mu.lock().await;
        let mut keys = if self.path.exists() {
            let content = tokio::fs::read_to_string(&self.path).await?;
            toml::from_str::<KeysFile>(&content)?
        } else {
            return Ok(false);
        };
        let before = keys.keys.len();
        keys.keys.retain(|k| k.key != key);
        let removed = before > keys.keys.len();
        if removed {
            self.write(&keys).await?;
        }
        Ok(removed)
    }

    /// 验证是否是有效的 admin key
    pub async fn validate_admin(&self, key: &str) -> Result<bool> {
        let keys = self.load().await?;
        Ok(keys.iter().any(|k| k.key == key && k.admin))
    }

    /// 验证是否是有效的隧道 key（非 admin）
    pub async fn validate_tunnel(&self, key: &str) -> Result<bool> {
        let keys = self.load().await?;
        Ok(keys.iter().any(|k| k.key == key && !k.admin))
    }

    /// 获取第一个 admin key（用于 Web 面板展示）
    pub async fn find_admin_key(&self) -> Result<Option<String>> {
        let keys = self.load().await?;
        Ok(keys.into_iter().find(|k| k.admin).map(|k| k.key))
    }

    /// 根据 API key 查找名称
    pub async fn find_name(&self, key: &str) -> Result<Option<String>> {
        let keys = self.load().await?;
        Ok(keys.iter().find(|k| k.key == key).map(|k| k.name.clone()))
    }

    async fn write(&self, keys: &KeysFile) -> Result<()> {
        let content = toml::to_string_pretty(keys).context("serialize keys")?;
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        tokio::fs::write(&self.path, content).await.context("write keys.toml")?;
        Ok(())
    }
}

/// 首次启动时自动生成初始 admin key
pub async fn ensure_initial_key(store: &KeyStore) -> Result<String> {
    let keys = store.load().await?;
    if keys.iter().any(|k| k.admin) {
        return Ok(String::new());
    }
    let api_key = gout_api::generate_api_key();
    let now: DateTime<Utc> = Utc::now();
    store
        .add(KeyEntry {
            key: api_key.clone(),
            name: "admin".into(),
            created_at: now.to_rfc3339(),
            admin: true,
        })
        .await?;
    Ok(api_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (KeyStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::new(dir.path());
        (store, dir)
    }

    fn make_entry(key: &str, name: &str, admin: bool) -> KeyEntry {
        KeyEntry {
            key: key.into(),
            name: name.into(),
            created_at: Utc::now().to_rfc3339(),
            admin,
        }
    }

    #[tokio::test]
    async fn test_empty_load() {
        let (store, _) = temp_store();
        assert!(store.load().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_add_and_load() {
        let (store, _) = temp_store();
        store.add(make_entry("sk-abc", "test", false)).await.unwrap();
        let keys = store.load().await.unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].key, "sk-abc");
    }

    #[tokio::test]
    async fn test_delete() {
        let (store, _) = temp_store();
        store.add(make_entry("sk-abc", "x", false)).await.unwrap();
        assert!(store.delete("sk-abc").await.unwrap());
        assert!(store.load().await.unwrap().is_empty());
        assert!(!store.delete("sk-nonexistent").await.unwrap());
    }

    #[tokio::test]
    async fn test_validate_admin() {
        let (store, _) = temp_store();
        store.add(make_entry("sk-admin", "admin", true)).await.unwrap();
        store.add(make_entry("sk-user", "user", false)).await.unwrap();

        assert!(store.validate_admin("sk-admin").await.unwrap());
        assert!(!store.validate_admin("sk-user").await.unwrap());

        assert!(!store.validate_tunnel("sk-admin").await.unwrap());
        assert!(store.validate_tunnel("sk-user").await.unwrap());
    }

    #[tokio::test]
    async fn test_find_name() {
        let (store, _) = temp_store();
        store.add(make_entry("sk-foo", "我的笔记本", false)).await.unwrap();
        assert_eq!(store.find_name("sk-foo").await.unwrap().unwrap(), "我的笔记本");
        assert!(store.find_name("sk-bar").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_ensure_initial_key_creates_admin() {
        let (store, _) = temp_store();
        let key = ensure_initial_key(&store).await.unwrap();
        assert!(key.starts_with("sk-"));
        assert!(store.validate_admin(&key).await.unwrap());

        // 第二次调用返回空
        assert!(ensure_initial_key(&store).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_persists_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = KeyStore::new(dir.path());
            store.add(make_entry("sk-persist", "p", false)).await.unwrap();
        }
        let store2 = KeyStore::new(dir.path());
        assert_eq!(store2.load().await.unwrap().len(), 1);
    }
}
