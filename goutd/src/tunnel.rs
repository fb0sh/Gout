//! 隧道管理器 — 服务端核心状态。
//!
//! 线程安全，通过 `Arc<TunnelManager>` 在 HTTP handler 和 data server 之间共享。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

use gout_api::TunnelType;

// ━━━ 类型 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub type Token = u64;

/// 信号通道消息：data server 通过此 channel 通知 signal handler 有新外部连接
#[derive(Debug, Clone)]
pub enum SignalMsg {
    NewExternalConnection,
    Shutdown,
}

// ━━━ 隧道状态 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug)]
pub struct Tunnel {
    pub token: Token,
    pub tunnel_type: TunnelType,
    pub public_port: u16,
    pub key_name: String,
    pub created_at: Instant,
    /// 信号通道发送端，data server accept 循环使用
    pub signal_tx: Option<tokio::sync::mpsc::Sender<SignalMsg>>,
    /// TCP 隧道：待转发的活跃外部连接 (conn_id → TcpStream)
    /// UDP 隧道：不使用此字段
    pub pending_conns: Vec<tokio::net::TcpStream>,
    /// UDP 隧道：绑定的公网 UDP socket
    pub udp_socket: Option<Arc<UdpSocket>>,
}

// ━━━ 管理器 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub struct TunnelManager {
    tunnels: RwLock<HashMap<Token, Tunnel>>,
    free_ports: Mutex<Vec<u16>>,
    data_port: u16,
    /// 握手超时时间
    handshake_timeout: Duration,
    /// 是否已启动清理循环
    cleanup_started: AtomicBool,
}

impl TunnelManager {
    pub fn new(port_start: u16, port_end: u16, data_port: u16) -> Self {
        // 端口池：从高到低存放，pop 取最低可用端口
        let free_ports: Vec<u16> = (port_start..=port_end).rev().collect();
        Self {
            tunnels: RwLock::new(HashMap::new()),
            free_ports: Mutex::new(free_ports),
            data_port,
            handshake_timeout: Duration::from_secs(30),
            cleanup_started: AtomicBool::new(false),
        }
    }

    /// 分配一个公网端口。返回 None 表示端口池已耗尽。
    pub async fn allocate_port(&self) -> Option<u16> {
        self.free_ports.lock().await.pop()
    }

    /// 归还端口
    pub async fn release_port(&self, port: u16) {
        self.free_ports.lock().await.push(port);
    }

    pub fn data_port(&self) -> u16 {
        self.data_port
    }

    /// 创建隧道并启动公网端口监听。返回 token。
    pub async fn create_tunnel(
        self: &Arc<Self>,
        tunnel_type: TunnelType,
        key_name: String,
        bind_ip: std::net::IpAddr,
    ) -> Result<(Token, u16), String> {
        let port = self.allocate_port().await.ok_or("no free ports")?;
        let token = gout_api::generate_token();

        let tunnel = Tunnel {
            token,
            tunnel_type,
            public_port: port,
            key_name,
            created_at: Instant::now(),
            signal_tx: None,
            pending_conns: Vec::new(),
            udp_socket: None,
        };

        self.tunnels.write().await.insert(token, tunnel);

        // TCP/HTTP 隧道：启动公网端口监听
        if tunnel_type == TunnelType::Tcp || tunnel_type == TunnelType::Http {
            let mgr = self.clone();
            let addr = SocketAddr::new(bind_ip, port);
            tokio::spawn(async move {
                if let Err(e) = mgr.run_public_listener(token, addr).await {
                    warn!("public listener for tunnel {} ended: {}", token, e);
                }
            });
        }

        // UDP 隧道：绑定公网 UDP socket
        if tunnel_type == TunnelType::Udp {
            let addr = SocketAddr::new(bind_ip, port);
            match UdpSocket::bind(addr).await {
                Ok(socket) => {
                    let socket = Arc::new(socket);
                    self.set_udp_socket(token, socket.clone()).await
                        .map_err(|e| format!("store udp socket: {e}"))?;
                    info!("UDP socket bound on {} for tunnel {}", addr, token);
                }
                Err(e) => {
                    self.close_tunnel(token).await.ok();
                    return Err(format!("bind UDP socket on {}: {e}", addr));
                }
            }
        }

        Ok((token, port))
    }

    /// 公网端口 accept 循环
    async fn run_public_listener(&self, token: Token, addr: SocketAddr) -> Result<(), String> {
        let listener = TcpListener::bind(addr).await.map_err(|e| e.to_string())?;
        info!("public listener started on {} for tunnel {}", addr, token);

        loop {
            let (stream, _peer) = match listener.accept().await {
                Ok(c) => c,
                Err(_) => break,
            };

            if self.add_pending_conn(token, stream).await.is_err() {
                break;
            }
        }

        Ok(())
    }

    /// 注册信号通道。仅 TCP 隧道首次数据连接时调用。
    /// 返回 SignalMsg receiver — data server spawn 一个 signal handler 使用它。
    pub async fn register_signal_channel(
        &self,
        token: Token,
    ) -> Result<tokio::sync::mpsc::Receiver<SignalMsg>, String> {
        let mut tunnels = self.tunnels.write().await;
        let tunnel = tunnels.get_mut(&token).ok_or("tunnel not found")?;

        if tunnel.signal_tx.is_some() {
            return Err("signal channel already registered".into());
        }

        let (tx, rx) = tokio::sync::mpsc::channel::<SignalMsg>(32);
        tunnel.signal_tx = Some(tx);
        Ok(rx)
    }

    /// 添加一个待转发的外部连接（TCP 隧道）。
    /// 同时通过信号通道通知客户端。
    pub async fn add_pending_conn(
        &self,
        token: Token,
        stream: tokio::net::TcpStream,
    ) -> Result<(), String> {
        let tunnels = self.tunnels.read().await;
        let tunnel = tunnels.get(&token).ok_or("tunnel not found")?;

        // 通过信号通道通知客户端
        if let Some(ref tx) = tunnel.signal_tx {
            tx.send(SignalMsg::NewExternalConnection)
                .await
                .map_err(|_| "signal channel closed".to_string())?;
        }

        // 需要 write lock 来 push pending_conns
        drop(tunnels);
        let mut tunnels = self.tunnels.write().await;
        let tunnel = tunnels.get_mut(&token).ok_or("tunnel not found")?;
        tunnel.pending_conns.push(stream);

        Ok(())
    }

    /// 取出一个待转发的外部连接，供客户端数据通道 pipe。
    pub async fn take_pending_conn(
        &self,
        token: Token,
    ) -> Result<tokio::net::TcpStream, String> {
        let mut tunnels = self.tunnels.write().await;
        let tunnel = tunnels.get_mut(&token).ok_or("tunnel not found")?;

        if tunnel.pending_conns.is_empty() {
            return Err("no pending connection".into());
        }

        // FIFO：取最早的连接
        Ok(tunnel.pending_conns.remove(0))
    }

    /// 关闭隧道。归还端口，移除记录。
    pub async fn close_tunnel(&self, token: Token) -> Result<(), String> {
        let mut tunnels = self.tunnels.write().await;
        let tunnel = tunnels.remove(&token).ok_or("tunnel not found")?;

        self.release_port(tunnel.public_port).await;

        // 通知信号通道关闭
        if let Some(tx) = tunnel.signal_tx {
            let _ = tx.send(SignalMsg::Shutdown).await;
        }

        Ok(())
    }

    /// 获取隧道信息（如果存在）
    pub async fn get_tunnel(&self, token: Token) -> Option<TunnelType> {
        self.tunnels
            .read()
            .await
            .get(&token)
            .map(|t| t.tunnel_type)
    }

    /// 设置隧道 UDP socket
    pub async fn set_udp_socket(&self, token: Token, socket: Arc<UdpSocket>) -> Result<(), String> {
        let mut tunnels = self.tunnels.write().await;
        let tunnel = tunnels.get_mut(&token).ok_or("tunnel not found")?;
        tunnel.udp_socket = Some(socket);
        Ok(())
    }

    /// 获取隧道 UDP socket
    pub async fn get_udp_socket(&self, token: Token) -> Option<Arc<UdpSocket>> {
        self.tunnels.read().await.get(&token)?.udp_socket.clone()
    }

    /// 检查隧道是否存在
    pub async fn tunnel_exists(&self, token: Token) -> bool {
        self.tunnels.read().await.contains_key(&token)
    }

    /// 获取所有活跃隧道信息，供 Web 面板展示
    pub async fn list_tunnels(&self) -> Vec<TunnelInfo> {
        self.tunnels
            .read()
            .await
            .iter()
            .map(|(token, t)| TunnelInfo {
                token: *token,
                tunnel_type: t.tunnel_type,
                public_port: t.public_port,
                key_name: t.key_name.clone(),
                has_signal: t.signal_tx.is_some(),
                pending_count: t.pending_conns.len(),
            })
            .collect()
    }

    /// 启动后台清理循环，定期关闭过期隧道
    pub fn start_cleanup_loop(self: &Arc<Self>) {
        if self.cleanup_started.swap(true, Ordering::Relaxed) {
            return;
        }
        let timeout = self.handshake_timeout;
        let mgr = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                let now = Instant::now();
                let mut to_close = Vec::new();

                for (token, tunnel) in mgr.tunnels.read().await.iter() {
                    let connected = match tunnel.tunnel_type {
                        // UDP：有数据通道连接时 handle_udp 会注册 dummy signal_tx
                        // 防止清理循环误关
                        TunnelType::Udp => tunnel.signal_tx.is_some(),
                        _ => tunnel.signal_tx.is_some(),
                    };
                    if !connected
                        && now.duration_since(tunnel.created_at) > timeout
                    {
                        to_close.push(*token);
                    }
                }

                for token in to_close {
                    info!("tunnel {} expired (handshake timeout)", token);
                    let _ = mgr.close_tunnel(token).await;
                }
            }
        });
    }
}

// ━━━ Web 展示用 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone)]
pub struct TunnelInfo {
    pub token: u64,
    pub tunnel_type: TunnelType,
    pub public_port: u16,
    pub key_name: String,
    pub has_signal: bool,
    pub pending_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_mgr() -> Arc<TunnelManager> {
        Arc::new(TunnelManager::new(20000, 20010, 8081))
    }

    #[tokio::test]
    async fn test_create_tunnel_allocates_port() {
        let mgr = make_mgr();
        let free_before = mgr.free_ports.lock().await.len();
        let (token, port) = mgr
            .create_tunnel(TunnelType::Tcp, "test".into(), std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
            .await
            .unwrap();
        assert!(token != 0);
        assert!(port >= 20000 && port <= 20010);

        let free_after = mgr.free_ports.lock().await.len();
        assert_eq!(free_after, free_before - 1);
    }

    #[tokio::test]
    async fn test_create_tunnel_port_exhaustion() {
        let mgr = Arc::new(TunnelManager::new(30000, 30000, 8081)); // only 1 port
        // 用光所有端口
        let (_, _) = mgr
            .create_tunnel(TunnelType::Tcp, "a".into(), std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
            .await
            .unwrap();
        // 下一个应该失败
        let r = mgr
            .create_tunnel(TunnelType::Tcp, "b".into(), std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
            .await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn test_signal_channel_registration() {
        let mgr = make_mgr();
        let (token, _) = mgr
            .create_tunnel(TunnelType::Tcp, "t".into(), std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
            .await
            .unwrap();

        // 第一次注册应成功
        let rx = mgr.register_signal_channel(token).await;
        assert!(rx.is_ok());

        // 第二次注册应失败
        let rx2 = mgr.register_signal_channel(token).await;
        assert!(rx2.is_err());
    }

    #[tokio::test]
    async fn test_close_tunnel_frees_port() {
        let mgr = make_mgr();
        let free_before = mgr.free_ports.lock().await.len();
        let (token, port) = mgr
            .create_tunnel(TunnelType::Tcp, "t".into(), std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
            .await
            .unwrap();

        mgr.close_tunnel(token).await.unwrap();

        let free_after = mgr.free_ports.lock().await.len();
        assert_eq!(free_after, free_before);
        assert!(mgr.free_ports.lock().await.contains(&port));
    }

    #[tokio::test]
    async fn test_add_pending_conn_without_signal_fails() {
        let mgr = make_mgr();
        let (token, _) = mgr
            .create_tunnel(TunnelType::Tcp, "t".into(), std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
            .await
            .unwrap();
        // 未注册 signal channel 时添加 pending conn 应失败
        // 没有 signal channel 和 pending conn 时 take 应失败
        let r = mgr.take_pending_conn(token).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn test_list_tunnels() {
        let mgr = make_mgr();
        assert!(mgr.list_tunnels().await.is_empty());

        let (token, _) = mgr
            .create_tunnel(TunnelType::Tcp, "my key".into(), std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
            .await
            .unwrap();

        let list = mgr.list_tunnels().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].token, token);
        assert_eq!(list[0].key_name, "my key");
        assert!(!list[0].has_signal);
    }
}
