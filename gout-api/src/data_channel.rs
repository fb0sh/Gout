//! 数据通道协议 — 握手、确认、双向 pipe。

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{decode_handshake, encode_handshake, TunnelType, STATUS_OK};

/// 握手错误
#[derive(Debug)]
pub enum HandshakeError {
    Io(std::io::Error),
    Rejected,
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

/// 客户端发起握手：发送 [token][tunnel_type]，等 1 字节状态码。
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

/// 服务端等待并解析握手，返回 (token, tunnel_type)。
pub async fn server_receive_handshake(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<(u64, TunnelType), HandshakeError> {
    let mut buf = [0u8; crate::HANDSHAKE_SIZE];
    stream.read_exact(&mut buf).await?;
    Ok(decode_handshake(&buf))
}

/// 服务端发送握手成功响应。
pub async fn server_accept(
    stream: &mut (impl AsyncWrite + Unpin),
) -> Result<(), std::io::Error> {
    stream.write_all(&[STATUS_OK]).await
}

/// 服务端发送握手拒绝响应 + 原因。
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
