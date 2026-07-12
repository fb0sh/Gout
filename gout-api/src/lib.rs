//! # Gout API
//!
//! 共享协议类型 + 客户端 SDK。
//!
//! - **协议类型**: TunnelType, 帧编解码, REST API 类型, token 生成
//! - **GoutClient**: 使用普通 api-key 进行隧道操作
//! - **GoutAdminClient**: 使用 admin key 进行管理操作

pub mod admin;
pub mod client;
pub mod data_channel;

// ━━━ 重新导出协议类型 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

mod proto;
pub use proto::*;
