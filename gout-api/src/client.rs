//! `GoutClient` — 使用普通 `api-key` 与 goutd 通信。
//!
//! 负责隧道的 CRUD 操作（创建、列出、删除）。
//! 数据通道的握手和转发由 [`data_channel`](crate::data_channel) 模块处理。

use crate::{CreateTunnelRequest, TunnelResponse};
use anyhow::{Context, Result};

/// 隧道操作客户端。
///
/// 使用服务端分配的普通 `api-key`（非 admin key）进行认证。
/// 客户端实例持有 HTTP 连接池，建议复用。
///
/// # 示例
///
/// ```no_run
/// # async fn doc() {
/// use gout_api::client::GoutClient;
/// use gout_api::TunnelType;
///
/// let gout = GoutClient::new("server.example.com:8080", "sk-your-key");
/// let tunnel = gout.create_tunnel(TunnelType::Tcp, 4000, None).await.unwrap();
/// gout.delete_tunnel(tunnel.token).await.unwrap();
/// # }
/// ```
pub struct GoutClient {
    inner: reqwest::Client,
    base: String,
    api_key: String,
}

impl GoutClient {
    /// 创建一个新的 `GoutClient`。
    ///
    /// # 参数
    ///
    /// * `server_addr` — 服务端地址，`host:port` 格式，如 `"example.com:8080"`
    /// * `api_key` — 普通隧道 API key
    pub fn new(server_addr: &str, api_key: &str) -> Self {
        Self {
            inner: reqwest::Client::new(),
            base: format!("http://{server_addr}"),
            api_key: api_key.to_string(),
        }
    }

    /// 创建一个新隧道。
    ///
    /// 返回 [`TunnelResponse`]，调用方据此建立数据通道连接。
    /// 数据通道的握手和 pipe 由 [`data_channel`](crate::data_channel) 模块提供。
    ///
    /// # 参数
    ///
    /// * `tunnel_type` — 隧道协议类型（TCP / UDP / HTTP）
    /// * `local_port` — 本地服务端口号
    /// * `remote_port` — 远端公网端口号（可选，`None` 由服务端自动分配）
    pub async fn create_tunnel(
        &self,
        tunnel_type: crate::TunnelType,
        local_port: u16,
        remote_port: Option<u16>,
    ) -> Result<TunnelResponse> {
        let resp = self
            .inner
            .post(format!("{}/api/v1/tunnels", self.base))
            .header("X-Api-Key", &self.api_key)
            .json(&CreateTunnelRequest {
                tunnel_type,
                local_port: Some(local_port),
                remote_port,
            })
            .send()
            .await
            .context("REST create tunnel failed")?;

        crate::parse_api_response(resp).await
    }

    /// 列出所有活跃隧道。
    ///
    /// 返回当前服务端上状态为 "waiting" 或 "active" 的隧道列表。
    /// 已关闭或已过期的隧道不会出现在列表中。
    pub async fn list_tunnels(&self) -> Result<Vec<crate::TunnelListEntry>> {
        let resp = self
            .inner
            .get(format!("{}/api/v1/tunnels", self.base))
            .header("X-Api-Key", &self.api_key)
            .send()
            .await
            .context("REST list tunnels failed")?;

        crate::parse_api_response(resp).await
    }

    /// 删除指定 token 的隧道。
    ///
    /// 服务端会关闭对应的公网端口监听并清理所有相关资源。
    pub async fn delete_tunnel(&self, token: u64) -> Result<()> {
        let resp = self
            .inner
            .delete(format!("{}/api/v1/tunnels/{}", self.base, token))
            .header("X-Api-Key", &self.api_key)
            .send()
            .await
            .context("REST delete tunnel failed")?;

        if !resp.status().is_success() {
            anyhow::bail!("delete tunnel failed: {}", resp.status());
        }
        Ok(())
    }

    /// 获取服务端地址（不含 `http://` 前缀）。
    pub fn server_addr(&self) -> &str {
        &self.base[7..]
    }

    /// 获取当前客户端使用的 API key。
    pub fn api_key(&self) -> &str {
        &self.api_key
    }
}
