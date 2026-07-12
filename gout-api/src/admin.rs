//! `GoutAdminClient` — 使用 admin key 管理 goutd。
//!
//! 负责 API key 的创建、列出和删除。
//! 需要服务端启动时打印的 `admin-api-key`。

use crate::{ApiResponse, CreateKeyRequest, CreateKeyResponse, KeyInfo};
use anyhow::{Context, Result};

/// 管理客户端。
///
/// 使用 `admin-api-key`（非普通隧道 key）进行认证。
/// 用于创建/删除普通隧道 key。
///
/// # 示例
///
/// ```no_run
/// # async fn doc() {
/// use gout_api::admin::GoutAdminClient;
///
/// let admin = GoutAdminClient::new("server.example.com:8080", "admin-key");
/// let key = admin.create_key("my laptop").await.unwrap();
/// println!("new key: {}", key.key);
/// # }
/// ```
pub struct GoutAdminClient {
    inner: reqwest::Client,
    base: String,
    admin_key: String,
}

impl GoutAdminClient {
    /// 创建一个新的 `GoutAdminClient`。
    ///
    /// # 参数
    ///
    /// * `server_addr` — 服务端地址，`host:port` 格式
    /// * `admin_key` — 服务端首次启动时打印到 stdout 的 admin API key
    pub fn new(server_addr: &str, admin_key: &str) -> Self {
        Self {
            inner: reqwest::Client::new(),
            base: format!("http://{server_addr}"),
            admin_key: admin_key.to_string(),
        }
    }

    /// 创建一个普通 API key。
    ///
    /// 该 key 可用于 [`GoutClient`](crate::client::GoutClient) 进行隧道操作。
    ///
    /// # 参数
    ///
    /// * `name` — key 的备注名称（如 `"my laptop"`）
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
    ///
    /// 返回的列表中不包含 admin key。
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
    ///
    /// 返回 `true` 表示删除成功，`false` 表示 key 不存在。
    /// 不允许删除 admin key。
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

    /// 获取 admin key 值。
    pub fn admin_key(&self) -> &str {
        &self.admin_key
    }
}
