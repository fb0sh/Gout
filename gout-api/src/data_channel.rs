//! 数据通道协议 — 握手、确认、双向 pipe。
//!
//! 提供客户端和服务端两端的数据通道握手函数，
//! 以及双向 TCP 数据转发（pipe）功能。

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{decode_handshake, encode_handshake, TunnelType, STATUS_OK};

/// 数据通道握手错误。
#[derive(Debug)]
pub enum HandshakeError {
    /// I/O 错误（连接断开、超时等）
    Io(std::io::Error),
    /// 服务端拒绝了握手（token 无效或隧道已关闭）
    Rejected,
    /// token 不存在或隧道已关闭
    NotFound,
}

impl std::fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Rejected => write!(f, "server rejected handshake"),
            Self::NotFound => write!(f, "unknown token or tunnel closed"),
        }
    }
}

impl std::error::Error for HandshakeError {}

impl From<std::io::Error> for HandshakeError {
    fn from(e: std::io::Error) -> Self { Self::Io(e) }
}

/// 客户端发起握手：发送 `[token: u64 BE][tunnel_type: u8]` 并等待服务端确认。
///
/// 成功返回 `Ok(())`，失败返回 [`HandshakeError`]。
pub async fn client_handshake(
    stream: &mut (impl AsyncWrite + AsyncRead + Unpin),
    token: u64,
    tunnel_type: TunnelType,
) -> Result<(), HandshakeError> {
    let buf = encode_handshake(token, tunnel_type);
    stream.write_all(&buf).await?;
    let mut status = [0u8; 1];
    stream.read_exact(&mut status).await?;
    if status[0] == STATUS_OK {
        Ok(())
    } else {
        Err(HandshakeError::Rejected)
    }
}

/// 服务端接收并解析客户端握手，返回 `(token, tunnel_type)`。
pub async fn server_receive_handshake(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<(u64, TunnelType), HandshakeError> {
    let mut buf = [0u8; crate::HANDSHAKE_SIZE];
    stream.read_exact(&mut buf).await?;
    Ok(decode_handshake(&buf))
}

/// 服务端发送握手成功响应（1 字节 `STATUS_OK`）。
pub async fn server_accept(
    stream: &mut (impl AsyncWrite + Unpin),
) -> Result<(), std::io::Error> {
    stream.write_all(&[STATUS_OK]).await
}

/// 服务端发送握手拒绝响应：`[STATUS_ERR][err_len: u16 BE][reason]`。
pub async fn server_reject(
    stream: &mut (impl AsyncWrite + Unpin),
    reason: &str,
) -> Result<(), std::io::Error> {
    let mut resp = vec![crate::STATUS_ERR];
    let msg_bytes = reason.as_bytes();
    resp.extend_from_slice(&(msg_bytes.len() as u16).to_be_bytes());
    resp.extend_from_slice(msg_bytes);
    stream.write_all(&resp).await
}

/// 双向 pipe 两个 TCP stream，任一方断开即结束。
///
/// 内部使用 `tokio::io::copy` 和 `tokio::select!` 实现双向转发。
pub async fn pipe_bidirectional(
    mut a: tokio::net::TcpStream,
    mut b: tokio::net::TcpStream,
) {
    let (mut ar, mut aw) = a.split();
    let (mut br, mut bw) = b.split();
    tokio::select! {
        _ = tokio::io::copy(&mut ar, &mut bw) => {}
        _ = tokio::io::copy(&mut br, &mut aw) => {}
    }
}
