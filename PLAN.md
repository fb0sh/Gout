# Gout — 轻量内网穿透工具 设计方案

## 1. 目标

类似 frp/rathole，但更轻量、更快上手。C/S 架构：

- **gout（客户端）**：一行命令开启隧道，配置自动保存到 `~/.goutrc`
- **goutd（服务端）**：公网服务器上运行，提供 Web 面板管理 key 和查看隧道

## 2. 架构总览

```
┌──────────────┐                          ┌─────────────────────┐
│    gout      │  REST API (HTTP)         │      goutd          │
│   (CLI)      │ ◄──────────────────────► │   (daemon)          │
│              │  认证 + 隧道增删查        │                     │
│  ~/.goutrc   │                          │   :8080  HTTP       │
│              │                          │   :8081  data       │
│              │  数据隧道 (raw TCP)       │                     │
│              │ ◄──────────────────────► │                     │
└──────────────┘                          └──────────┬──────────┘
                                                     │
                                                     ▼  :10000 (公网端口)
                                                ┌──────────┐
                                                │ 外部用户  │
                                                └──────────┘
```

**两个端口：**
| 端口 | 协议 | 用途 |
|------|------|------|
| 8080 | HTTP | REST API + Web 面板（仅监听 localhost） |
| 8081 | Raw TCP | 隧道数据通道 |

**Web 面板安全：** 仅监听 `127.0.0.1`，通过 SSH 端口转发访问：`ssh -L 8080:localhost:8080 server`

## 3. 用户故事

```bash
# ─── 服务端 ───
# 启动，首次自动生成初始 API key
goutd
# → Initial API key: sk-xxxxxxxxxxxx  (save this!)

# SSH 端口转发访问 Web 面板
ssh -L 8080:localhost:8080 server
# 浏览器打开 http://localhost:8080

# ─── 客户端 ───
gout login server.example.com:8080 sk-xxxxxxxxxxxx
gout tcp 4000    # 本地 4000 → 公网。多开隧道 = 多开终端
gout udp 5353
gout http 3000   # v0.1 就是 tcp 别名
```

## 4. 组件设计

### 4.1 gout-proto（共享库）

- `TunnelType`: Tcp | Udp | Http
- 二进制帧编解码（握手、数据帧）
- REST API 类型
- Token 生成

### 4.2 gout（CLI 客户端）

```
gout/src/
├── main.rs        → 入口
├── cli.rs         → clap 命令: login, tcp, udp, http
└── config.rs      → ~/.goutrc 读写
```

**单进程单隧道：** 每个 `gout tcp 4000` 启动一个独立进程，自带事件循环。多隧道 = 多终端。

**隧道生命周期（`gout tcp 4000`）：**

```
1. 读取 ~/.goutrc → server_addr, api_key
2. POST /api/v1/tunnels  { type: "tcp", local_port: 4000 }
   ← { tunnel_id, token, public_port, data_port }
3. 打印 "Tunnel: server:10000 -> localhost:4000"
4. 进入事件循环：
   a. 等待服务端通知（新外部连接到来）
      服务端如何通知？→ 数据通道上接收 [token] 握手 + [status] 响应
   b. 收到通知后：
      - TCP 连接数据端口 → 发送 [token: u64 BE]
      - 收到 OK → 开始双向 pipe
      - 本地 connect(localhost:4000)
      - pipe: 数据通道 ↔ 本地连接
      - 如果本地 connect 失败 → 发错误通知给服务端
5. Ctrl+C → 关闭连接 → DELETE 隧道
```

**UDP 隧道（`gout udp 5353`）：**
```
同上，但只维持一条持久数据通道。
服务端在公网端口收到 UDP 包 → 封装 [len:u16][data] 通过数据通道发出
客户端收到 → 转发到本地 UDP socket
反向同理。
```

### 4.3 goutd（服务端守护进程）

```
goutd/src/
├── main.rs          → 入口，启动 HTTP + data server
├── config.rs        → 服务端配置
├── api/
│   ├── mod.rs       → axum Router
│   ├── auth.rs      → X-Api-Key header 验证中间件
│   └── tunnels.rs   → 隧道 CRUD
├── web/
│   └── mod.rs       → Web 面板路由 + askama 模板上下文
├── store.rs         → keys.toml 读写
├── tunnel.rs        → TunnelManager
└── templates/
    ├── base.html     → 页面骨架
    ├── dashboard.html → 活跃隧道列表
    └── keys.html     → Key 管理
```

**REST API：**

| 方法 | 路径 | 说明 | 认证 |
|------|------|------|------|
| GET | `/dashboard` | 隧道总览页面 | — |
| GET | `/keys` | Key 管理页面 | — |
| POST | `/api/v1/keys` | 创建 key `{"name":"笔记本"}` | — |
| GET | `/api/v1/keys` | 列出所有 key | — |
| DELETE | `/api/v1/keys/:key` | 删除 key | — |
| POST | `/api/v1/tunnels` | 创建隧道 | X-Api-Key |
| DELETE | `/api/v1/tunnels/:token` | 删除隧道 | X-Api-Key |

**TunnelManager：**
- 端口分配：空闲端口池（Vec<u16>），分配时 pop，释放时 push
- 隧道映射：`token → TunnelState`（token = u64 随机数，认证用）
- 外部连接处理：accept → 通知客户端 → 分配 conn_id → pipe 数据
- 清理：数据通道断开 → 关闭外部连接 → 归还端口 → 删除隧道

## 5. 数据通道协议

### 5.1 握手

```
Client → Server:  [token: u64 BE]    (8 bytes)
Server → Client:  [status: u8]       (1 byte)
                   0x01 = OK
                   0x00 = Error, 后跟 [err_len: u16][err_msg]
```

**token** 是创建隧道时服务端生成的 u64 随机数，替代明文 tunnel_id 防止隧道劫持。

### 5.2 TCP 隧道数据转发

```
每个外部连接 → 服务端通知客户端 → 客户端新建一条数据通道：

Client → Server:  [token: u64 BE]       ← 握手
Server → Client:  [0x01]                ← OK

然后双向 pipe：
  Server ↔ Client:  raw bytes（无帧头，纯数据转发）
```

一条数据通道 = 一个外部连接。没有多路复用，没有 conn_id。

### 5.3 UDP 隧道数据转发

```
一条持久数据通道，承载 UDP 数据报：

  帧格式：[len: u16 BE][data: len bytes]
  
  len = 0 表示隧道关闭信号。
```

### 5.4 HTTP 隧道

v0.1 就是 TCP 别名，`gout http 3000` = `gout tcp 3000`。

## 6. 持久化存储

### 6.1 ~/.goutrc

```toml
[server]
addr = "server.example.com:8080"
api_key = "sk-xxxxxxxxxxxx"
```

### 6.2 {data_dir}/keys.toml

首次启动时如果文件不存在且没有 key，自动生成一个初始 key 并打印到 stdout。

```toml
[[keys]]
key = "sk-xxxxxxxxxxxx"
name = "auto-generated"
created_at = "2026-07-12T20:00:00Z"
```

### 6.3 隧道状态

不持久化。goutd 重启后所有隧道清空，客户端需重新连接。

## 7. 项目文件树

```
gout/
├── Cargo.toml              # workspace
├── .gitignore
├── PLAN.md
├── gout-proto/
│   ├── Cargo.toml
│   └── src/lib.rs          # TunnelType, 帧编解码, API 类型, token
├── gout/
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs
│       ├── cli.rs          # login, tcp, udp, http
│       └── config.rs       # ~/.goutrc
└── goutd/
    ├── Cargo.toml
    ├── src/
    │   ├── main.rs
    │   ├── config.rs
    │   ├── api/
    │   │   ├── mod.rs
    │   │   ├── auth.rs
    │   │   └── tunnels.rs
    │   ├── web/
    │   │   └── mod.rs
    │   ├── store.rs
    │   └── tunnel.rs
    └── templates/
        ├── base.html
        ├── dashboard.html
        └── keys.html
```

## 8. 依赖清单

| Crate | 用途 |
|-------|------|
| tokio | 异步运行时 |
| axum | HTTP 框架 |
| askama | 服务端模板 |
| clap | CLI 参数解析 |
| reqwest | HTTP 客户端 |
| serde + toml | 序列化 / 配置 |
| tower-http | CORS 中间件 |
| tracing | 日志 |
| uuid | API key 生成 |
| chrono | 时间戳 |
| anyhow | 错误处理 |
| dirs | ~ 目录路径 |
| rand | token 随机数 |

## 9. 实现阶段

### Phase 1 — 骨架 ✅
- [x] Workspace + 三个 crate
- [x] proto 库（TunnelType, API 类型, token）

### Phase 2 — 服务端
- [ ] goutd 启动 + 配置
- [ ] Key 管理（keys.toml + REST API + Web 面板）
- [ ] TunnelManager（端口池 + 隧道状态 + 外部连接处理）
- [ ] 数据通道 TCP server

### Phase 3 — 客户端
- [ ] CLI 命令（login, tcp, udp, http）
- [ ] ~/.goutrc 读写
- [ ] REST API 调用
- [ ] 隧道事件循环 + 数据转发

### Phase 4 — 集成
- [ ] 端到端 TCP 隧道
- [ ] UDP 隧道
- [ ] 错误处理

## 10. 与 rathole 的差异

| | rathole | gout |
|---|---------|------|
| 控制面 | 加密通道内嵌 | REST API |
| 认证 | Noise / TLS | API key + token |
| 配置 | TOML 文件热加载 | CLI 命令即时创建 |
| 数据通道 | 与控制耦合在一起 | 独立 TCP |
| 数据复用 | 每连接一条 channel | 每连接一条 TCP |
| 管理面板 | 无 | Web 面板（localhost） |
| 上手 | 需写配置 | `gout login` → `gout tcp 4000` |

## 11. Grilling 决策记录

| # | 决策 | 结论 |
|---|------|------|
| 1 | 数据通道复用 vs 每连接一条 | **每连接一条**（简单） |
| 2 | UDP 隧道方案 | 一条持久数据通道 + 帧封装 |
| 3 | HTTP 隧道语义 | v0.1 = TCP 别名 |
| 4 | 数据通道加密 | v0.1 不加密，Web 面板 only localhost |
| 5 | 端口分配策略 | 空闲端口池 Vec<u16> |
| 6 | 多隧道支持 | 一个进程一个隧道 |
| 7 | 客户端断连清理 | TCP 断开自动触发 |
| 8 | 本地服务未启动 | 客户端发送错误通知 |
| 9 | 数据通道认证 | token (u64 随机数) |
