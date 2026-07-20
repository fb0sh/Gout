//! # Gout API
//!
//! [Gout](https://github.com/fb0sh/Gout) 的 Rust SDK。
//!
//! 包含共享协议类型和两个客户端：
//!
//! | 模块 | 用途 | 认证 |
//! |------|------|------|
//! | [`client`] | 隧道操作（创建/列出/删除） | 普通 `api-key` |
//! | [`admin`] | 管理操作（创建/列出/删除 API key） | `admin-api-key` |
//! | [`data_channel`] | 数据通道握手和 TCP 转发（底层协议） | token 认证 |
//!
//! # 快速开始
//!
//! ```no_run
//! # async fn doc() {
//! use gout_api::client::GoutClient;
//! use gout_api::admin::GoutAdminClient;
//! use gout_api::TunnelType;
//!
//! let gout = GoutClient::new("server:8080", "sk-xxx");
//! let tun = gout.create_tunnel(TunnelType::Tcp, 4000, None).await.unwrap();
//! gout.delete_tunnel(tun.token).await.unwrap();
//!
//! let admin = GoutAdminClient::new("server:8080", "admin-key");
//! let key = admin.create_key("my laptop").await.unwrap();
//! # }
//! ```

use anyhow::Context;

pub mod admin;
pub mod client;
pub mod data_channel;

mod proto;
pub use proto::*;

// ━━━ 内部 HTTP 辅助 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 解析 REST API 响应体，提取 `data` 字段。
/// 非 2xx 时从 `error` 字段构造错误信息。
pub(crate) async fn parse_api_response<T>(
    resp: reqwest::Response,
) -> anyhow::Result<T>
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let status = resp.status();
    if !status.is_success() {
        let body: ApiResponse<T> = resp
            .json()
            .await
            .unwrap_or(ApiResponse {
                success: false,
                data: None,
                error: Some(status.to_string()),
            });
        anyhow::bail!("{}", body.error.unwrap_or_default());
    }
    let body: ApiResponse<T> = resp.json().await?;
    body.data.context("no data in response")
}
