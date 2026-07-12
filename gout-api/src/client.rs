//! GoutClient — 使用普通 api-key 与 goutd 通信。
//!
//! 负责隧道 CRUD、数据通道握手（数据层 I/O 由调用方处理）。

use crate::{ApiResponse, CreateTunnelRequest, TunnelResponse};
use anyhow::{Context, Result};

/// 隧道操作客户端。
///
/// 使用普通 `api-key`（非 admin key）。
/// 数据通道的建立和维护由调用方自行处理。
pub struct GoutClient {
    inner: reqwest::Client,
    base: String,
    api_key: String,
}

impl GoutClient {
    /// 创建客户端。
    ///
    /// - `server_addr` — `host:port` 格式，如 `example.com:8080`
    /// - `api_key` — 普通隧道 key
    pub fn new(server_addr: &str, api_key: &str) -> Self {
        Self {
            inner: reqwest::Client::new(),
            base: format!("http://{server_addr}"),
            api_key: api_key.to_string(),
        }
    }

    /// 创建隧道。
    ///
    /// 返回 `TunnelResponse`，调用方据此建立数据通道连接。
    pub async fn create_tunnel(
        &self,
        tunnel_type: crate::TunnelType,
        local_port: u16,
    ) -> Result<TunnelResponse> {
        let resp = self
            .inner
            .post(format!("{}/api/v1/tunnels", self.base))
            .header("X-Api-Key", &self.api_key)
            .json(&CreateTunnelRequest {
                tunnel_type,
                local_port: Some(local_port),
            })
            .send()
            .await
            .context("REST create tunnel failed")?;

        if !resp.status().is_success() {
            let api_resp: ApiResponse<TunnelResponse> = resp
                .json()
                .await
                .context("parse error response")?;
            anyhow::bail!("server error: {}", api_resp.error.unwrap_or_default());
        }

        let api_resp: ApiResponse<TunnelResponse> = resp
            .json()
            .await
            .context("parse success response")?;
        api_resp.data.context("no tunnel data in response")
    }

    /// 删除隧道。
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

    /// 服务端地址（供调用方获取 data_port 或连接数据通道时使用）
    pub fn server_addr(&self) -> &str {
        &self.base[7..] // strip "http://"
    }

    /// api-key
    pub fn api_key(&self) -> &str {
        &self.api_key
    }
}
