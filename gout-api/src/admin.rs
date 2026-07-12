//! GoutAdminClient — 使用 admin key 管理 goutd。
//!
//! 负责 API key 的增删查。

use crate::{ApiResponse, CreateKeyRequest, CreateKeyResponse, KeyInfo};
use anyhow::{Context, Result};

/// 管理客户端。
///
/// 使用 `admin-api-key`（非普通隧道 key）。
pub struct GoutAdminClient {
    inner: reqwest::Client,
    base: String,
    admin_key: String,
}

impl GoutAdminClient {
    /// 创建管理客户端。
    ///
    /// - `server_addr` — `host:port`
    /// - `admin_key` — 服务端启动时打印的 admin key
    pub fn new(server_addr: &str, admin_key: &str) -> Self {
        Self {
            inner: reqwest::Client::new(),
            base: format!("http://{server_addr}"),
            admin_key: admin_key.to_string(),
        }
    }

    /// 创建一个普通 API key。
    pub async fn create_key(&self, name: &str) -> Result<CreateKeyResponse> {
        let resp = self
            .inner
            .post(format!("{}/api/v1/keys", self.base))
            .header("X-Admin-Key", &self.admin_key)
            .json(&CreateKeyRequest { name: name.into() })
            .send()
            .await
            .context("REST create key failed")?;

        if !resp.status().is_success() {
            let j: ApiResponse<CreateKeyResponse> = resp.json().await?;
            anyhow::bail!("{}", j.error.unwrap_or_default());
        }

        let j: ApiResponse<CreateKeyResponse> = resp.json().await?;
        j.data.context("no key data")
    }

    /// 列出所有普通 API key。
    pub async fn list_keys(&self) -> Result<Vec<KeyInfo>> {
        let resp = self
            .inner
            .get(format!("{}/api/v1/keys", self.base))
            .header("X-Admin-Key", &self.admin_key)
            .send()
            .await
            .context("REST list keys failed")?;

        let j: ApiResponse<Vec<KeyInfo>> = resp.json().await?;
        Ok(j.data.unwrap_or_default())
    }

    /// 删除一个 API key。
    pub async fn delete_key(&self, key: &str) -> Result<bool> {
        let resp = self
            .inner
            .delete(format!("{}/api/v1/keys/{}", self.base, key))
            .header("X-Admin-Key", &self.admin_key)
            .send()
            .await
            .context("REST delete key failed")?;

        let j: ApiResponse<()> = resp.json().await?;
        Ok(j.success)
    }

    /// admin key 值
    pub fn admin_key(&self) -> &str {
        &self.admin_key
    }
}
