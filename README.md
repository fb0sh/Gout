# Gout

受 frp 和 rathole 启发的轻量级内网穿透工具。一行命令将本地服务暴露到公网。

## 特点

- **零配置客户端**：`gout login server:8080 key` → `gout tcp 4000`，无需手写配置文件
- **Web 管理面板**：在浏览器中管理 API key、查看活跃隧道、添加备注
- **REST 控制面**：隧道通过 HTTP API 创建和销毁，不依赖配置文件热加载
- **多协议支持**：TCP、UDP 和 HTTP（v0.1 中 HTTP 是 TCP 别名）
- **信号通道架构**：每个隧道一条轻量控制连接，通知客户端有新的外部连接；每条连接使用独立的数据通道
- **Token 认证**：每个隧道分配一个随机 64 位 token，用于数据通道身份验证
- **端口池管理**：基于空闲池的端口分配，无碎片问题
- **自动清理**：客户端 30 秒内未完成数据通道握手时隧道自动过期；控制连接断开时全部清理

## 架构

```
gout (CLI)  ──REST──►  goutd :8080 (HTTP API + Web 面板)
     │                      │
     └──数据通道──────►  goutd :8081 (原始 TCP)
```

项目是一个 Cargo workspace，包含三个 crate：
- **gout-proto** — 共享协议类型、二进制编码、REST API 类型
- **gout** — CLI 客户端：login/tcp/udp/http 子命令；读取 `~/.goutrc`
- **goutd** — 服务端守护进程：axum HTTP 服务器 + tokio 数据通道 TCP 服务器

## 快速开始

```bash
# 服务端（公网 VPS）
goutd
# → Initial API key: sk-xxxxxxxxxxxx
# → Web 面板: http://127.0.0.1:8080（通过 SSH 访问：ssh -L 8080:localhost:8080 server）

# 客户端
gout login server.example.com:8080 sk-xxxxxxxxxxxx
gout tcp 4000     # 将本地 localhost:4000 暴露到公网
```

## License

MIT
