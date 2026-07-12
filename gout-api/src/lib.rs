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
//! let tun = gout.create_tunnel(TunnelType::Tcp, 4000).await.unwrap();
//! gout.delete_tunnel(tun.token).await.unwrap();
//!
//! let admin = GoutAdminClient::new("server:8080", "admin-key");
//! let key = admin.create_key("my laptop").await.unwrap();
//! # }
//! ```

pub mod admin;
pub mod client;
pub mod data_channel;

mod proto;
pub use proto::*;
