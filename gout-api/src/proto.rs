//! Gout 共享协议类型 — 供 `gout` 和 `goutd` 共用。

use serde::{Deserialize, Serialize};

// ━━━ 隧道类型 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelType {
    Tcp = 0,
    Udp = 1,
    Http = 2,
}

impl TunnelType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
            Self::Http => "http",
        }
    }

    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Tcp),
            1 => Some(Self::Udp),
            2 => Some(Self::Http),
            _ => None,
        }
    }

    pub fn to_u8(self) -> u8 {
        self as u8
    }

    /// 从字符串解析（gout CLI 使用）
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

pub const HANDSHAKE_SIZE: usize = 9;
pub const STATUS_OK: u8 = 0x01;
pub const STATUS_ERR: u8 = 0x00;
pub const SIGNAL_NEW_CONN: u8 = 0x02;
pub const UDP_FRAME_HEADER: usize = 2;

/// 编码握手帧
pub fn encode_handshake(token: u64, tunnel_type: TunnelType) -> [u8; HANDSHAKE_SIZE] {
    let mut buf = [0u8; HANDSHAKE_SIZE];
    buf[..8].copy_from_slice(&token.to_be_bytes());
    buf[8] = tunnel_type.to_u8();
    buf
}

/// 解码握手帧，返回 (token, tunnel_type)
pub fn decode_handshake(buf: &[u8; HANDSHAKE_SIZE]) -> (u64, TunnelType) {
    let token = u64::from_be_bytes(buf[..8].try_into().unwrap());
    let tt = TunnelType::from_u8(buf[8]).unwrap_or(TunnelType::Tcp);
    (token, tt)
}

/// 编码 UDP 帧
pub fn encode_udp_frame(data: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(2 + data.len());
    frame.extend_from_slice(&(data.len() as u16).to_be_bytes());
    frame.extend_from_slice(data);
    frame
}

/// 解码 UDP 帧头
pub fn decode_udp_header(buf: &[u8; UDP_FRAME_HEADER]) -> u16 {
    u16::from_be_bytes(*buf)
}

// ━━━ REST API 类型 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiResponse<T: Serialize> {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self { success: true, data: Some(data), error: None }
    }
    pub fn err(msg: impl Into<String>) -> Self {
        Self { success: false, data: None, error: Some(msg.into()) }
    }
}

pub fn api_ok() -> ApiResponse<()> {
    ApiResponse { success: true, data: Some(()), error: None }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateTunnelRequest {
    #[serde(rename = "type")]
    pub tunnel_type: TunnelType,
    #[serde(default)]
    pub local_port: Option<u16>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TunnelResponse {
    #[serde(with = "serde_u64_str")]
    pub token: u64,
    pub public_port: u16,
    pub data_port: u16,
    pub tunnel_type: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateKeyRequest {
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateKeyResponse {
    pub key: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KeyInfo {
    pub key: String,
    pub name: String,
}

// ━━━ token / API key 生成 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use rand::Rng;

pub fn generate_token() -> u64 {
    rand::thread_rng().gen()
}

pub fn generate_api_key() -> String {
    let id = uuid::Uuid::new_v4();
    format!("sk-{}", id.to_string().replace('-', "")[..24].to_string())
}

// ━━━ 序列化辅助 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 将 u64 序列化为 JSON 字符串，避免 JavaScript 精度丢失
pub mod serde_u64_str {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(val: &u64, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(val)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ━━━ TunnelType ━━━

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
        // 非法值回退到 Tcp
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

    // ━━━ 数据通道协议 ━━━

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

    // ━━━ Token / API key ━━━

    #[test]
    fn generate_token_is_nonzero() {
        // 极不可能生成0
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
        assert_eq!(key.len(), 27); // "sk-" + 24 hex chars
        // 应该只包含十六进制字符
        let hex_part = &key[3..];
        assert!(hex_part.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // ━━━ REST API 类型 ━━━

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
        };
        let j = serde_json::to_string(&req).unwrap();
        assert!(j.contains("\"type\":\"tcp\""));
        assert!(j.contains("\"local_port\":4000"));

        // 反序列化
        let parsed: CreateTunnelRequest = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.tunnel_type, TunnelType::Tcp);
        assert_eq!(parsed.local_port, Some(4000));
    }

    #[test]
    fn create_tunnel_request_default_local_port() {
        let j = r#"{"type":"udp"}"#;
        let req: CreateTunnelRequest = serde_json::from_str(j).unwrap();
        assert_eq!(req.tunnel_type, TunnelType::Udp);
        assert_eq!(req.local_port, None);
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
