//! Gout 共享协议类型 — 供 `gout` 和 `goutd` 共用。
//!
//! 包含隧道类型枚举、数据通道帧编解码、REST API 请求/响应类型、以及 token/API key 生成函数。

use serde::{Deserialize, Serialize};

// ━━━ 隧道类型 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 隧道传输协议类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelType {
    /// TCP 隧道 — 每个外部连接使用一条独立数据通道
    Tcp = 0,
    /// UDP 隧道 — 一条持久数据通道承载帧封装的数据报
    Udp = 1,
    /// HTTP 隧道 — v0.1 等价于 TCP
    Http = 2,
}

impl TunnelType {
    /// 返回协议类型的字符串标识（`"tcp"` / `"udp"` / `"http"`）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
            Self::Http => "http",
        }
    }

    /// 从 u8 解析（用于二进制握手帧解码），返回 `None` 表示未知值。
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Tcp),
            1 => Some(Self::Udp),
            2 => Some(Self::Http),
            _ => None,
        }
    }

    /// 将类型编码为 u8（用于二进制握手帧编码）。
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    /// 从字符串解析（`"tcp"` / `"udp"` / `"http"`），未知值回退到 `Tcp`。
    pub fn parse(s: &str) -> Self {
        match s {
            "tcp" => Self::Tcp,
            "udp" => Self::Udp,
            "http" => Self::Http,
            _ => Self::Tcp,
        }
    }
}

impl std::fmt::Display for TunnelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl Serialize for TunnelType {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TunnelType {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        match s.as_str() {
            "tcp" => Ok(Self::Tcp),
            "udp" => Ok(Self::Udp),
            "http" => Ok(Self::Http),
            _ => Err(serde::de::Error::custom(format!("unknown tunnel type: {s}"))),
        }
    }
}

// ━━━ 数据通道协议 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 握手帧字节数：`[token: u64 BE]` + `[tunnel_type: u8]` = 9 字节。
pub const HANDSHAKE_SIZE: usize = 9;

/// 握手成功状态码。
pub const STATUS_OK: u8 = 0x01;

/// 握手失败状态码。
pub const STATUS_ERR: u8 = 0x00;

/// 信号通道通知字节：服务端→客户端，表示有新的外部连接。
pub const SIGNAL_NEW_CONN: u8 = 0x02;

/// UDP 帧头字节数：`[len: u16 BE]` = 2 字节。
pub const UDP_FRAME_HEADER: usize = 2;

/// 编码客户端握手帧。
///
/// 输出 `[token: u64 BE][tunnel_type: u8]` 共 9 字节。
pub fn encode_handshake(token: u64, tunnel_type: TunnelType) -> [u8; HANDSHAKE_SIZE] {
    let mut buf = [0u8; HANDSHAKE_SIZE];
    buf[..8].copy_from_slice(&token.to_be_bytes());
    buf[8] = tunnel_type.to_u8();
    buf
}

/// 解码客户端握手帧，返回 `(token, tunnel_type)`。
pub fn decode_handshake(buf: &[u8; HANDSHAKE_SIZE]) -> (u64, TunnelType) {
    let token = u64::from_be_bytes(buf[..8].try_into().unwrap());
    let tt = TunnelType::from_u8(buf[8]).unwrap_or(TunnelType::Tcp);
    (token, tt)
}

/// 编码 UDP 帧：`[len: u16 BE][data]`。
pub fn encode_udp_frame(data: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(2 + data.len());
    frame.extend_from_slice(&(data.len() as u16).to_be_bytes());
    frame.extend_from_slice(data);
    frame
}

/// 隧道列表条目，由 `GET /api/v1/tunnels` 返回。
#[derive(Debug, Serialize, Deserialize)]
pub struct TunnelListEntry {
    /// 隧道 token（序列化为字符串以避免 JavaScript 精度丢失）
    #[serde(with = "serde_u64_str")]
    pub token: u64,
    /// 隧道类型字符串
    pub tunnel_type: String,
    /// 公网端口
    pub public_port: u16,
    /// 创建此隧道的 API key 名称
    pub key_name: String,
    /// 客户端是否已连接（TCP signal 或 UDP data channel）
    pub connected: bool,
    /// 待转发的挂起连接数
    pub pending_count: usize,
}

/// 解码 UDP 帧头，返回 payload 长度。
pub fn decode_udp_header(buf: &[u8; UDP_FRAME_HEADER]) -> u16 {
    u16::from_be_bytes(*buf)
}

// ━━━ REST API 类型 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 通用 REST API 响应外壳。
///
/// 所有端点均返回此结构，`success` 表示操作是否成功。
#[derive(Debug, Serialize, Deserialize)]
pub struct ApiResponse<T: Serialize> {
    /// 操作是否成功
    pub success: bool,
    /// 成功时携带的数据（可选）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    /// 失败时的错误信息（可选）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl<T: Serialize> ApiResponse<T> {
    /// 构造成功响应。
    pub fn ok(data: T) -> Self {
        Self { success: true, data: Some(data), error: None }
    }
    /// 构造失败响应。
    pub fn err(msg: impl Into<String>) -> Self {
        Self { success: false, data: None, error: Some(msg.into()) }
    }
}

/// 构造一个无数据的成功响应 `{"success": true, "data": null}`。
pub fn api_ok() -> ApiResponse<()> {
    ApiResponse { success: true, data: Some(()), error: None }
}

/// 创建隧道请求体。
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateTunnelRequest {
    /// 隧道协议类型（JSON key 为 `type`）
    #[serde(rename = "type")]
    pub tunnel_type: TunnelType,
    /// 本地端口号（可选，供服务端记录）
    #[serde(default)]
    pub local_port: Option<u16>,
    /// 远程端口号（可选，指定服务端公网端口）
    #[serde(default)]
    pub remote_port: Option<u16>,
}

/// 创建隧道响应体。
#[derive(Debug, Serialize, Deserialize)]
pub struct TunnelResponse {
    /// 隧道 token（序列化为字符串以避免 JavaScript 精度丢失）
    #[serde(with = "serde_u64_str")]
    pub token: u64,
    /// 服务端分配的公网端口
    pub public_port: u16,
    /// 数据通道端口
    pub data_port: u16,
    /// 隧道类型字符串
    pub tunnel_type: String,
}

/// 创建 API key 请求体。
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateKeyRequest {
    /// key 备注名称
    pub name: String,
}

/// 创建 API key 响应体。
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateKeyResponse {
    /// 新生成的 API key
    pub key: String,
    /// 备注名称
    pub name: String,
}

/// API key 基本信息（列表展示时使用）。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KeyInfo {
    /// API key 值
    pub key: String,
    /// 备注名称
    pub name: String,
}

// ━━━ token / API key 生成 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use rand::Rng;

/// 生成一个随机的 64 位隧道 token。
pub fn generate_token() -> u64 {
    rand::thread_rng().gen()
}

/// 生成一个随机的 API key，格式为 `sk-` + 24 位十六进制字符。
pub fn generate_api_key() -> String {
    let id = uuid::Uuid::new_v4();
    format!("sk-{}", id.to_string().replace('-', "")[..24].to_string())
}

// ━━━ 序列化辅助 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 将 `u64` 序列化为 JSON 字符串的 serde 辅助模块。
///
/// 使用方式：`#[serde(with = "serde_u64_str")]`
///
/// JavaScript 的 `Number` 类型只能精确表示 2^53 以内的整数，
/// 而 Gout 的 token 是 64 位随机数，可能超出此范围。
/// 将 token 序列化为字符串可避免 JSON 解析时的精度丢失。
pub mod serde_u64_str {
    use serde::{Deserialize, Deserializer, Serializer};

    /// 将 `u64` 序列化为 JSON 字符串。
    pub fn serialize<S: Serializer>(val: &u64, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(val)
    }

    /// 从 JSON 字符串反序列化为 `u64`。
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tunnel_type_from_u8_roundtrip() {
        for (u, expected) in [(0, TunnelType::Tcp), (1, TunnelType::Udp), (2, TunnelType::Http)] {
            let tt = TunnelType::from_u8(u).unwrap();
            assert_eq!(tt, expected);
            assert_eq!(tt.to_u8(), u);
        }
    }

    #[test]
    fn tunnel_type_from_u8_out_of_range() {
        assert!(TunnelType::from_u8(3).is_none());
        assert!(TunnelType::from_u8(255).is_none());
    }

    #[test]
    fn tunnel_type_as_str() {
        assert_eq!(TunnelType::Tcp.as_str(), "tcp");
        assert_eq!(TunnelType::Udp.as_str(), "udp");
        assert_eq!(TunnelType::Http.as_str(), "http");
    }

    #[test]
    fn tunnel_type_display() {
        assert_eq!(format!("{}", TunnelType::Tcp), "tcp");
        assert_eq!(format!("{}", TunnelType::Udp), "udp");
        assert_eq!(format!("{}", TunnelType::Http), "http");
    }

    #[test]
    fn tunnel_type_parse() {
        assert_eq!(TunnelType::parse("tcp"), TunnelType::Tcp);
        assert_eq!(TunnelType::parse("udp"), TunnelType::Udp);
        assert_eq!(TunnelType::parse("http"), TunnelType::Http);
        assert_eq!(TunnelType::parse("bluetooth"), TunnelType::Tcp);
    }

    #[test]
    fn tunnel_type_serde_json() {
        let j = serde_json::to_string(&TunnelType::Tcp).unwrap();
        assert_eq!(j, "\"tcp\"");
        let tt: TunnelType = serde_json::from_str("\"udp\"").unwrap();
        assert_eq!(tt, TunnelType::Udp);
    }

    #[test]
    fn tunnel_type_serde_unknown_errs() {
        let r: Result<TunnelType, _> = serde_json::from_str("\"bluetooth\"");
        assert!(r.is_err());
    }

    #[test]
    fn handshake_roundtrip() {
        let token = 0xDEAD_BEEF_CAFE_F00D;
        for tt in [TunnelType::Tcp, TunnelType::Udp, TunnelType::Http] {
            let encoded = encode_handshake(token, tt);
            assert_eq!(encoded.len(), HANDSHAKE_SIZE);
            let (decoded_token, decoded_tt) = decode_handshake(&encoded);
            assert_eq!(decoded_token, token);
            assert_eq!(decoded_tt, tt);
        }
    }

    #[test]
    fn handshake_token_zero() {
        let encoded = encode_handshake(0, TunnelType::Tcp);
        let (token, tt) = decode_handshake(&encoded);
        assert_eq!(token, 0);
        assert_eq!(tt, TunnelType::Tcp);
    }

    #[test]
    fn udp_frame_roundtrip() {
        let payload = b"hello UDP";
        let frame = encode_udp_frame(payload);
        assert_eq!(frame.len(), 2 + payload.len());
        let mut header = [0u8; UDP_FRAME_HEADER];
        header.copy_from_slice(&frame[..2]);
        let len = decode_udp_header(&header);
        assert_eq!(len as usize, payload.len());
        assert_eq!(&frame[2..], payload);
    }

    #[test]
    fn udp_frame_empty_payload() {
        let frame = encode_udp_frame(&[]);
        assert_eq!(frame.len(), 2);
        let mut header = [0u8; UDP_FRAME_HEADER];
        header.copy_from_slice(&frame[..2]);
        assert_eq!(decode_udp_header(&header), 0);
    }

    #[test]
    fn constants() {
        assert_eq!(HANDSHAKE_SIZE, 9);
        assert_eq!(STATUS_OK, 0x01);
        assert_eq!(STATUS_ERR, 0x00);
        assert_eq!(SIGNAL_NEW_CONN, 0x02);
        assert_eq!(UDP_FRAME_HEADER, 2);
    }

    #[test]
    fn generate_token_is_nonzero() {
        for _ in 0..100 {
            let t = generate_token();
            if t == 0 {
                panic!("generated zero token");
            }
        }
    }

    #[test]
    fn generate_api_key_format() {
        let key = generate_api_key();
        assert!(key.starts_with("sk-"));
        assert_eq!(key.len(), 27);
        let hex_part = &key[3..];
        assert!(hex_part.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn api_ok_response() {
        let r = api_ok();
        assert!(r.success);
        assert!(r.data.is_some());
        assert!(r.error.is_none());
    }

    #[test]
    fn api_err_response() {
        let r: ApiResponse<()> = ApiResponse::err("something broke");
        assert!(!r.success);
        assert!(r.data.is_none());
        assert_eq!(r.error.unwrap(), "something broke");
    }

    #[test]
    fn create_tunnel_request_serde() {
        let req = CreateTunnelRequest {
            tunnel_type: TunnelType::Tcp,
            local_port: Some(4000),
            remote_port: None,
        };
        let j = serde_json::to_string(&req).unwrap();
        assert!(j.contains("\"type\":\"tcp\""));
        assert!(j.contains("\"local_port\":4000"));
        let parsed: CreateTunnelRequest = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.tunnel_type, TunnelType::Tcp);
        assert_eq!(parsed.local_port, Some(4000));
        assert_eq!(parsed.remote_port, None);
    }

    #[test]
    fn create_tunnel_request_with_remote_port() {
        let req = CreateTunnelRequest {
            tunnel_type: TunnelType::Tcp,
            local_port: Some(4000),
            remote_port: Some(10080),
        };
        let j = serde_json::to_string(&req).unwrap();
        assert!(j.contains("\"remote_port\":10080"));
        let parsed: CreateTunnelRequest = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.remote_port, Some(10080));
    }

    #[test]
    fn create_tunnel_request_default_local_port() {
        let j = r#"{"type":"udp"}"#;
        let req: CreateTunnelRequest = serde_json::from_str(j).unwrap();
        assert_eq!(req.tunnel_type, TunnelType::Udp);
        assert_eq!(req.local_port, None);
        assert_eq!(req.remote_port, None);
    }

    #[test]
    fn tunnel_response_serde() {
        let resp = TunnelResponse {
            token: 42,
            public_port: 10001,
            data_port: 8081,
            tunnel_type: "tcp".into(),
        };
        let j = serde_json::to_string(&resp).unwrap();
        let parsed: TunnelResponse = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.token, 42);
        assert_eq!(parsed.public_port, 10001);
        assert_eq!(parsed.data_port, 8081);
        assert_eq!(parsed.tunnel_type, "tcp");
    }
}
