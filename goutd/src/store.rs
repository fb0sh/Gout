/// Key 持久化存储 — TOML 文件读写。
///
/// 文件格式 ({data_dir}/keys.toml):
/// ```toml
/// [[keys]]
/// key = "sk-xxxxxxxxxxxx"
/// name = "我的笔记本"
/// created_at = "2026-07-12T20:00:00Z"
/// ```

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

    /// 加载所有 key。如果文件不存在返回空列表。
    pub async fn load(&self) -> Result<Vec<KeyEntry>> {
        let _lock = self.mu.lock().await;

        if !self.path.exists() {
            return Ok(vec![]);
        }

        let content =
            tokio::fs::read_to_string(&self.path).await.context("read keys.toml")?;
        let keys: KeysFile = toml::from_str(&content).context("parse keys.toml")?;
        Ok(keys.keys)
    }

    /// 添加一个 key 并持久化。
    pub async fn add(&self, entry: KeyEntry) -> Result<()> {
        let _lock = self.mu.lock().await;

        let mut keys = if self.path.exists() {
            let content =
                tokio::fs::read_to_string(&self.path).await.context("read keys.toml")?;
            toml::from_str::<KeysFile>(&content).context("parse keys.toml")?
        } else {
            KeysFile::default()
        };

        keys.keys.push(entry);
        self.write(&keys).await
    }

    /// 删除一个 key 并持久化。
    pub async fn delete(&self, key: &str) -> Result<bool> {
        let _lock = self.mu.lock().await;

        let mut keys = if self.path.exists() {
            let content =
                tokio::fs::read_to_string(&self.path).await.context("read keys.toml")?;
            toml::from_str::<KeysFile>(&content).context("parse keys.toml")?
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

    /// 验证 API key 是否有效
    pub async fn validate(&self, key: &str) -> Result<bool> {
        let keys = self.load().await?;
        Ok(keys.iter().any(|k| k.key == key))
    }

    /// 根据 API key 查找名称
    pub async fn find_name(&self, key: &str) -> Result<Option<String>> {
        let keys = self.load().await?;
        Ok(keys.iter().find(|k| k.key == key).map(|k| k.name.clone()))
    }

    async fn write(&self, keys: &KeysFile) -> Result<()> {
        let content = toml::to_string_pretty(keys).context("serialize keys")?;
        tokio::fs::write(&self.path, content)
            .await
            .context("write keys.toml")?;
        Ok(())
    }
}

/// 首次启动时自动生成初始 key
pub async fn ensure_initial_key(store: &KeyStore) -> Result<String> {
    let keys = store.load().await?;
    if keys.is_empty() {
        let api_key = gout_proto::generate_api_key();
        let now: DateTime<Utc> = Utc::now();
        store
            .add(KeyEntry {
                key: api_key.clone(),
                name: "auto-generated".into(),
                created_at: now.to_rfc3339(),
            })
            .await?;
        Ok(api_key)
    } else {
        Ok(String::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (KeyStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::new(dir.path());
        (store, dir)
    }

    #[tokio::test]
    async fn test_empty_load() {
        let (store, _dir) = temp_store();
        let keys = store.load().await.unwrap();
        assert!(keys.is_empty());
    }

    #[tokio::test]
    async fn test_add_and_load() {
        let (store, _dir) = temp_store();
        store
            .add(KeyEntry {
                key: "sk-abc".into(),
                name: "test key".into(),
                created_at: Utc::now().to_rfc3339(),
            })
            .await
            .unwrap();

        let keys = store.load().await.unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].key, "sk-abc");
        assert_eq!(keys[0].name, "test key");
    }

    #[tokio::test]
    async fn test_add_multiple() {
        let (store, _dir) = temp_store();
        for i in 0..3 {
            store
                .add(KeyEntry {
                    key: format!("sk-{i}"),
                    name: format!("key {i}"),
                    created_at: Utc::now().to_rfc3339(),
                })
                .await
                .unwrap();
        }
        let keys = store.load().await.unwrap();
        assert_eq!(keys.len(), 3);
    }

    #[tokio::test]
    async fn test_delete_exists() {
        let (store, _dir) = temp_store();
        store
            .add(KeyEntry {
                key: "sk-abc".into(),
                name: "x".into(),
                created_at: Utc::now().to_rfc3339(),
            })
            .await
            .unwrap();

        let removed = store.delete("sk-abc").await.unwrap();
        assert!(removed);

        let keys = store.load().await.unwrap();
        assert!(keys.is_empty());
    }

    #[tokio::test]
    async fn test_delete_not_found() {
        let (store, _dir) = temp_store();
        let removed = store.delete("sk-nonexistent").await.unwrap();
        assert!(!removed);
    }

    #[tokio::test]
    async fn test_validate() {
        let (store, _dir) = temp_store();
        store
            .add(KeyEntry {
                key: "sk-valid".into(),
                name: "v".into(),
                created_at: Utc::now().to_rfc3339(),
            })
            .await
            .unwrap();

        assert!(store.validate("sk-valid").await.unwrap());
        assert!(!store.validate("sk-wrong").await.unwrap());
    }

    #[tokio::test]
    async fn test_find_name() {
        let (store, _dir) = temp_store();
        store
            .add(KeyEntry {
                key: "sk-foo".into(),
                name: "我的笔记本".into(),
                created_at: Utc::now().to_rfc3339(),
            })
            .await
            .unwrap();

        let name = store.find_name("sk-foo").await.unwrap();
        assert_eq!(name.unwrap(), "我的笔记本");

        let missing = store.find_name("sk-bar").await.unwrap();
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn test_ensure_initial_key_when_empty() {
        let (store, _dir) = temp_store();
        let key = ensure_initial_key(&store).await.unwrap();
        assert!(key.starts_with("sk-"));
        assert!(!key.is_empty());

        // 第二次调用应返回空
        let second = ensure_initial_key(&store).await.unwrap();
        assert!(second.is_empty());
    }

    #[tokio::test]
    async fn test_persists_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::new(dir.path());
        store
            .add(KeyEntry {
                key: "sk-persist".into(),
                name: "p".into(),
                created_at: Utc::now().to_rfc3339(),
            })
            .await
            .unwrap();
        drop(store); // 显式 drop

        // 新建 store 读同一文件
        let store2 = KeyStore::new(dir.path());
        let keys = store2.load().await.unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].key, "sk-persist");
    }
}
