//! 隧道管理器 — 服务端核心状态。
//!
//! 端口分配策略：PortAllocator 只负责生成候选端口，不判断端口是否空闲。
//! 真正可用性由操作系统的 bind() 决定。AddrInUse 时自动换下一个端口。

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

use gout_api::TunnelType;

// ━━━ 类型 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub type Token = u64;

#[derive(Debug, Clone)]
pub enum SignalMsg {
    NewExternalConnection,
    Shutdown,
}

// ━━━ 端口分配器 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 端口分配器。
///
/// 只负责在配置范围内选择候选端口，不判断端口是否空闲。
/// 真正可用性由调用的 bind() 决定。
#[derive(Debug)]
pub struct PortAllocator {
    start: u16,
    end: u16,
    cursor: u16,
    /// 当前正在尝试绑定但尚未确认的端口
    candidates: HashSet<u16>,
    /// 已成功绑定并确认分配的端口
    allocated: HashSet<u16>,
}

impl PortAllocator {
    pub fn new(start: u16, end: u16) -> Self {
        Self {
            start,
            end,
            cursor: start,
            candidates: HashSet::new(),
            allocated: HashSet::new(),
        }
    }

    /// 获取下一个候选端口。返回 None 表示范围内无可用端口。
    pub fn next_candidate(&mut self) -> Option<u16> {
        let start = self.cursor;
        loop {
            if !self.candidates.contains(&self.cursor)
                && !self.allocated.contains(&self.cursor)
            {
                let port = self.cursor;
                self.advance_cursor();
                self.candidates.insert(port);
                return Some(port);
            }
            self.advance_cursor();
            if self.cursor == start {
                return None; // 绕了一圈，全占了
            }
        }
    }

    /// 标记候选端口为已确认（bind 成功）。
    pub fn confirm(&mut self, port: u16) {
        self.candidates.remove(&port);
        self.allocated.insert(port);
    }

    /// 归还候选端口（bind 返回 AddrInUse，非本进程占用）。
    pub fn reject(&mut self, port: u16) {
        self.candidates.remove(&port);
        // 不移入 allocated，也不放回 free pool——该端口已被外部进程占用
    }

    /// 释放已确认端口（tunnel 关闭）。
    pub fn release(&mut self, port: u16) {
        self.candidates.remove(&port);
        self.allocated.remove(&port);
        // 下次 cursor 扫描能重新选中它
    }

    /// 候选端口数（用于测试）
    pub fn candidate_count(&self) -> usize {
        self.candidates.len()
    }

    /// 已确认端口数（用于测试）
    pub fn allocated_count(&self) -> usize {
        self.allocated.len()
    }

    /// 范围内总端口数
    pub fn total(&self) -> u16 {
        self.end - self.start + 1
    }

    fn advance_cursor(&mut self) {
        self.cursor += 1;
        if self.cursor > self.end {
            self.cursor = self.start;
        }
    }
}

// ━━━ 隧道状态 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug)]
pub struct Tunnel {
    pub token: Token,
    pub tunnel_type: TunnelType,
    pub public_port: u16,
    pub key_name: String,
    pub created_at: Instant,
    pub connected: bool,
    pub signal_tx: Option<tokio::sync::mpsc::Sender<SignalMsg>>,
    pub pending_conns: Vec<TcpStream>,
    pub udp_socket: Option<Arc<UdpSocket>>,
}

// ━━━ 管理器 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub struct TunnelManager {
    tunnels: RwLock<HashMap<Token, Tunnel>>,
    allocator: Mutex<PortAllocator>,
    data_port: u16,
    handshake_timeout: Duration,
    cleanup_started: AtomicBool,
}

impl TunnelManager {
    pub fn new(port_start: u16, port_end: u16, data_port: u16) -> Self {
        Self {
            tunnels: RwLock::new(HashMap::new()),
            allocator: Mutex::new(PortAllocator::new(port_start, port_end)),
            data_port,
            handshake_timeout: Duration::from_secs(30),
            cleanup_started: AtomicBool::new(false),
        }
    }

    pub fn data_port(&self) -> u16 {
        self.data_port
    }

    /// 创建隧道。
    ///
    /// 如果 `remote_port` 指定，直接 bind 该端口（失败则报错）；
    /// 否则循环尝试 PortAllocator 候选端口直到 bind 成功或耗尽。
    pub async fn create_tunnel(
        self: &Arc<Self>,
        tunnel_type: TunnelType,
        key_name: String,
        bind_ip: std::net::IpAddr,
        remote_port: Option<u16>,
    ) -> Result<(Token, u16), String> {
        match tunnel_type {
            TunnelType::Tcp | TunnelType::Http => {
                self.create_tcp_tunnel(tunnel_type, key_name, bind_ip, remote_port).await
            }
            TunnelType::Udp => {
                self.create_udp_tunnel(key_name, bind_ip, remote_port).await
            }
        }
    }

    /// TCP/HTTP 隧道：指定端口直接 bind，否则 bind 循环 → spawn listener
    async fn create_tcp_tunnel(
        self: &Arc<Self>,
        tunnel_type: TunnelType,
        key_name: String,
        bind_ip: std::net::IpAddr,
        remote_port: Option<u16>,
    ) -> Result<(Token, u16), String> {
        let token = gout_api::generate_token();

        let (listener, port) = if let Some(port) = remote_port {
            // 直接尝试 bind 指定端口，失败即报错
            let addr = SocketAddr::new(bind_ip, port);
            let listener = TcpListener::bind(addr).await.map_err(|e| {
                if e.kind() == std::io::ErrorKind::AddrInUse {
                    format!("remote port {port} is already in use")
                } else {
                    format!("bind TCP on {bind_ip}:{port}: {e}")
                }
            })?;
            // bind 成功，登记到 allocator 的 allocated 集合
            self.allocator.lock().await.confirm(port);
            (listener, port)
        } else {
            // bind 循环：尝试候选端口，AddrInUse 则换下一个
            loop {
                let port = {
                    let mut alloc = self.allocator.lock().await;
                    let port = alloc.next_candidate().ok_or("no free ports")?;
                    port
                };

                let addr = SocketAddr::new(bind_ip, port);
                match TcpListener::bind(addr).await {
                    Ok(l) => {
                        // bind 成功，确认分配
                        self.allocator.lock().await.confirm(port);
                        break (l, port);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                        // 端口已被占用，放弃该候选
                        self.allocator.lock().await.reject(port);
                        continue;
                    }
                    Err(e) => {
                        self.allocator.lock().await.reject(port);
                        return Err(format!("bind TCP on {bind_ip}: {e}"));
                    }
                }
            }
        };

        let tunnel = Tunnel {
            token,
            tunnel_type,
            public_port: port,
            key_name,
            created_at: Instant::now(),
            connected: false,
            signal_tx: None,
            pending_conns: Vec::new(),
            udp_socket: None,
        };

        self.tunnels.write().await.insert(token, tunnel);

        // 启动 accept 循环（传入已绑定的 listener，避免二次 bind）
        let mgr = self.clone();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Err(e) = mgr.run_public_listener(token, listener).await {
                warn!("public listener for tunnel {} ended: {}", token, e);
            }
        });
        info!("TCP listener started on {} for tunnel {}", addr, token);

        Ok((token, port))
    }

    /// UDP 隧道：指定端口直接 bind，否则 bind 循环 → 存储 socket
    async fn create_udp_tunnel(
        self: &Arc<Self>,
        key_name: String,
        bind_ip: std::net::IpAddr,
        remote_port: Option<u16>,
    ) -> Result<(Token, u16), String> {
        let token = gout_api::generate_token();

        let (socket, port) = if let Some(port) = remote_port {
            // 直接尝试 bind 指定端口，失败即报错
            let addr = SocketAddr::new(bind_ip, port);
            let socket = UdpSocket::bind(addr).await.map_err(|e| {
                if e.kind() == std::io::ErrorKind::AddrInUse {
                    format!("remote port {port} is already in use")
                } else {
                    format!("bind UDP on {bind_ip}:{port}: {e}")
                }
            })?;
            // bind 成功，登记到 allocator 的 allocated 集合
            self.allocator.lock().await.confirm(port);
            (Arc::new(socket), port)
        } else {
            loop {
                let port = {
                    let mut alloc = self.allocator.lock().await;
                    let port = alloc.next_candidate().ok_or("no free ports")?;
                    port
                };

                let addr = SocketAddr::new(bind_ip, port);
                match UdpSocket::bind(addr).await {
                    Ok(s) => {
                        self.allocator.lock().await.confirm(port);
                        break (Arc::new(s), port);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                        self.allocator.lock().await.reject(port);
                        continue;
                    }
                    Err(e) => {
                        self.allocator.lock().await.reject(port);
                        return Err(format!("bind UDP on {bind_ip}: {e}"));
                    }
                }
            }
        };

        let tunnel = Tunnel {
            token,
            tunnel_type: TunnelType::Udp,
            public_port: port,
            key_name,
            created_at: Instant::now(),
            connected: false,
            signal_tx: None,
            pending_conns: Vec::new(),
            udp_socket: None,
        };

        self.tunnels.write().await.insert(token, tunnel);

        // 存储 UDP socket 供 handle_udp 使用
        self.set_udp_socket(token, socket.clone()).await
            .map_err(|e| format!("store udp socket: {e}"))?;
        info!("UDP socket bound on {}:{} for tunnel {}", bind_ip, port, token);

        Ok((token, port))
    }

    /// 公网端口 accept 循环（接收预绑定的 listener）
    async fn run_public_listener(
        &self,
        token: Token,
        listener: TcpListener,
    ) -> Result<(), String> {
        let addr = listener.local_addr().map_err(|e| e.to_string())?;
        info!("public listener running on {} for tunnel {}", addr, token);

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

    // ─── 信号通道 ─────────────────────────────────────────────

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
        tunnel.connected = true;
        Ok(rx)
    }

    // ─── 外部连接管理 ─────────────────────────────────────────

    pub async fn add_pending_conn(
        &self,
        token: Token,
        stream: TcpStream,
    ) -> Result<(), String> {
        let tunnels = self.tunnels.read().await;
        let tunnel = tunnels.get(&token).ok_or("tunnel not found")?;

        if let Some(ref tx) = tunnel.signal_tx {
            tx.send(SignalMsg::NewExternalConnection)
                .await
                .map_err(|_| "signal channel closed".to_string())?;
        }

        drop(tunnels);
        let mut tunnels = self.tunnels.write().await;
        let tunnel = tunnels.get_mut(&token).ok_or("tunnel not found")?;
        tunnel.pending_conns.push(stream);

        Ok(())
    }

    pub async fn take_pending_conn(
        &self,
        token: Token,
    ) -> Result<TcpStream, String> {
        let mut tunnels = self.tunnels.write().await;
        let tunnel = tunnels.get_mut(&token).ok_or("tunnel not found")?;

        if tunnel.pending_conns.is_empty() {
            return Err("no pending connection".into());
        }

        Ok(tunnel.pending_conns.remove(0))
    }

    /// 关闭隧道。归还端口，移除记录。
    pub async fn close_tunnel(&self, token: Token) -> Result<(), String> {
        let mut tunnels = self.tunnels.write().await;
        let tunnel = tunnels.remove(&token).ok_or("tunnel not found")?;

        // 归还端口到分配器
        self.allocator.lock().await.release(tunnel.public_port);

        if let Some(tx) = tunnel.signal_tx {
            let _ = tx.send(SignalMsg::Shutdown).await;
        }

        Ok(())
    }

    // ─── 查询 ─────────────────────────────────────────────────

    pub async fn is_connected(&self, token: Token) -> Option<bool> {
        self.tunnels.read().await.get(&token).map(|t| t.connected)
    }

    pub async fn mark_connected(&self, token: Token) -> Result<(), String> {
        let mut tunnels = self.tunnels.write().await;
        let tunnel = tunnels.get_mut(&token).ok_or("tunnel not found")?;
        tunnel.connected = true;
        Ok(())
    }

    pub async fn set_udp_socket(&self, token: Token, socket: Arc<UdpSocket>) -> Result<(), String> {
        let mut tunnels = self.tunnels.write().await;
        let tunnel = tunnels.get_mut(&token).ok_or("tunnel not found")?;
        tunnel.udp_socket = Some(socket);
        Ok(())
    }

    pub async fn get_udp_socket(&self, token: Token) -> Option<Arc<UdpSocket>> {
        self.tunnels.read().await.get(&token)?.udp_socket.clone()
    }

    pub async fn tunnel_exists(&self, token: Token) -> bool {
        self.tunnels.read().await.contains_key(&token)
    }

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
                connected: t.connected,
                pending_count: t.pending_conns.len(),
            })
            .collect()
    }

    // ─── 清理循环 ─────────────────────────────────────────────

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
                    if !tunnel.connected
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
    pub connected: bool,
    pub pending_count: usize,
}

impl TunnelInfo {
    pub fn to_list_entry(&self) -> gout_api::TunnelListEntry {
        gout_api::TunnelListEntry {
            token: self.token,
            tunnel_type: self.tunnel_type.as_str().to_string(),
            public_port: self.public_port,
            key_name: self.key_name.clone(),
            connected: self.connected,
            pending_count: self.pending_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_mgr() -> Arc<TunnelManager> {
        Arc::new(TunnelManager::new(20000, 20010, 8081))
    }

    // ━━━ PortAllocator 纯同步测试 ━━━━━━━━━━━━━━━━━━━━━━━━━━━

    #[test]
    fn allocator_returns_sequential_ports() {
        let mut a = PortAllocator::new(100, 102);
        assert_eq!(a.next_candidate(), Some(100));
        assert_eq!(a.next_candidate(), Some(101));
        assert_eq!(a.next_candidate(), Some(102));
        assert_eq!(a.next_candidate(), None);
    }

    #[test]
    fn allocator_confirm_removes_from_candidate() {
        let mut a = PortAllocator::new(100, 100);
        let p = a.next_candidate().unwrap();
        assert_eq!(a.candidate_count(), 1);
        a.confirm(p);
        assert_eq!(a.candidate_count(), 0);
        assert_eq!(a.allocated_count(), 1);
        // 已确认的端口不再分配
        assert_eq!(a.next_candidate(), None);
    }

    #[test]
    fn allocator_reject_frees_port_for_retry() {
        let mut a = PortAllocator::new(100, 100);
        let p = a.next_candidate().unwrap(); // 100
        a.reject(p);
        // reject 移出 candidates，端口重新可用
        assert_eq!(a.next_candidate(), Some(100));
    }

    #[test]
    fn allocator_confirm_then_release() {
        let mut a = PortAllocator::new(100, 101);
        let a1 = a.next_candidate().unwrap(); // 100
        let a2 = a.next_candidate().unwrap(); // 101
        a.confirm(a1);
        a.confirm(a2);
        assert_eq!(a.next_candidate(), None);
        a.release(a1);
        // 释放后应能再次分配
        assert_eq!(a.next_candidate(), Some(100));
    }

    #[test]
    fn allocator_exhaustion() {
        let mut a = PortAllocator::new(100, 100); // 1 port
        let p = a.next_candidate().unwrap();
        a.confirm(p);
        assert_eq!(a.next_candidate(), None);
    }

    #[test]
    fn allocator_skips_allocated_ports_in_scan() {
        let mut a = PortAllocator::new(100, 103);
        let p1 = a.next_candidate().unwrap(); // 100
        a.confirm(p1);
        // cursor 在 101，扫描应跳过 100
        assert_eq!(a.next_candidate(), Some(101));
        a.confirm(101);
        assert_eq!(a.next_candidate(), Some(102));
        a.confirm(102);
        assert_eq!(a.next_candidate(), Some(103));
        a.confirm(103);
        assert_eq!(a.next_candidate(), None);
    }

    #[test]
    fn allocator_wraps_around() {
        let mut a = PortAllocator::new(100, 102);
        assert_eq!(a.next_candidate(), Some(100));
        assert_eq!(a.next_candidate(), Some(101));
        assert_eq!(a.next_candidate(), Some(102));
        assert_eq!(a.next_candidate(), None);
        a.release(100);
        // 释放后 cursor 已回到 100
        assert_eq!(a.next_candidate(), Some(100));
    }

    #[test]
    fn allocator_total() {
        let a = PortAllocator::new(100, 199);
        assert_eq!(a.total(), 100);
    }

    // ━━━ TunnelManager 集成测试 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    #[tokio::test]
    async fn test_create_tunnel_allocates_port() {
        let mgr = make_mgr();
        let (token, port) = mgr
            .create_tunnel(TunnelType::Tcp, "test".into(), std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), None)
            .await
            .unwrap();
        assert!(token != 0);
        assert!(port >= 20000 && port <= 20010);

        // 端口已确认分配
        let alloc = mgr.allocator.lock().await;
        assert_eq!(alloc.allocated_count(), 1);
        assert!(alloc.allocated.contains(&port));
    }

    #[tokio::test]
    async fn test_create_tunnel_port_exhaustion() {
        let mgr = Arc::new(TunnelManager::new(30000, 30000, 8081));
        let (_, _) = mgr
            .create_tunnel(TunnelType::Tcp, "a".into(), std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), None)
            .await
            .unwrap();
        let r = mgr
            .create_tunnel(TunnelType::Tcp, "b".into(), std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), None)
            .await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn test_signal_channel_registration() {
        let mgr = make_mgr();
        let (token, _) = mgr
            .create_tunnel(TunnelType::Tcp, "t".into(), std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), None)
            .await
            .unwrap();

        let rx = mgr.register_signal_channel(token).await;
        assert!(rx.is_ok());

        let rx2 = mgr.register_signal_channel(token).await;
        assert!(rx2.is_err());
    }

    #[tokio::test]
    async fn test_close_tunnel_frees_port() {
        let mgr = make_mgr();
        let (token, _port) = mgr
            .create_tunnel(TunnelType::Tcp, "t".into(), std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), None)
            .await
            .unwrap();

        mgr.close_tunnel(token).await.unwrap();

        // 端口已释放，应可再次分配
        let alloc = mgr.allocator.lock().await;
        assert_eq!(alloc.allocated_count(), 0);
    }

    #[tokio::test]
    async fn test_add_pending_conn_without_signal_fails() {
        let mgr = make_mgr();
        let (token, _) = mgr
            .create_tunnel(TunnelType::Tcp, "t".into(), std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), None)
            .await
            .unwrap();
        let r = mgr.take_pending_conn(token).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn test_list_tunnels() {
        let mgr = make_mgr();
        assert!(mgr.list_tunnels().await.is_empty());

        let (token, _) = mgr
            .create_tunnel(TunnelType::Tcp, "my key".into(), std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), None)
            .await
            .unwrap();

        let list = mgr.list_tunnels().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].token, token);
        assert_eq!(list[0].key_name, "my key");
        assert!(!list[0].connected);
    }
}
