# Gout

[![crates.io](https://img.shields.io/crates/v/gout.svg)](https://crates.io/crates/gout)
[![crates.io](https://img.shields.io/crates/v/goutd.svg)](https://crates.io/crates/goutd)
[![crates.io](https://img.shields.io/crates/v/gout-api.svg)](https://crates.io/crates/gout-api)
[![MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

受 frp 和 rathole 启发的轻量级内网穿透工具。一行命令将本地服务暴露到公网。

## 特点

- **零配置客户端**：`gout server set <name> <addr> <key>` → `gout tcp 4000`，无需手写配置文件
- **多 Server 支持**：`gout server set prod <addr> <key>`，`gout tcp 4444 -s prod`，`gout server default <name>` 切换默认
- **Web 管理面板**：在浏览器中管理 API key、查看活跃隧道、添加备注
- **REST 控制面**：隧道通过 HTTP API 创建和销毁，不依赖配置文件热加载
- **多协议支持**：TCP、UDP 和 HTTP（HTTP 当前等价于 TCP）
- **后台运行**：`gout tcp 4000 -d` 将隧道放入后台，`gout kill 4000` 停止
- **日志查看**：`gout log 4000` 查看后台日志，`gout log 4000 -f` 实时跟踪
- **本地列表**：`gout list` 显示本机所有运行中的后台隧道
- **信号通道架构**：TCP 隧道每条一个信号通道 + 每条外部连接一个独立数据通道
- **UDP 隧道**：一条持久数据通道承载帧封装的数据报，双向转发
- **Token 认证**：每个隧道分配一个随机 64 位 token，用于数据通道身份验证
- **端口分配**：游标扫描 + 操作系统 bind() 确认，AddrInUse 自动换端口，默认可用 22,768 个端口；支持 `-r` 指定远程端口
- **自动清理**：客户端 30 秒内未完成数据通道握手时隧道自动过期；控制连接断开时全部清理

## 架构

```
gout (CLI)  ──REST──►  goutd :8080 (HTTP API + Web 面板)
     │                      │
     └──数据通道──────►  goutd :8081 (原始 TCP)
```

项目是一个 Cargo workspace，包含三个 crate：
- **gout-api** — Rust SDK：协议类型、`GoutClient`（隧道操作）、`GoutAdminClient`（管理操作）、`data_channel`（握手/pipe）
- **gout** — CLI 客户端：tcp/udp/http 子命令 + server/tunnel 管理（ls/log/kill/server）；配置存储在 `~/.gout/config.toml`
- **goutd** — 服务端守护进程：axum HTTP 服务器 + tokio 数据通道 TCP 服务器

## 安装

### 从 crates.io

```bash
cargo install goutd    # 服务端
cargo install gout     # 客户端
```

### 从 GitHub Releases

从 [Releases 页面](https://github.com/fb0sh/Gout/releases) 下载对应平台的二进制（Linux x86_64 / ARM64 / i686、macOS Apple Silicon / Intel、Windows x64），无需安装 Rust。

### 从源码编译
```bash
git clone https://github.com/fb0sh/Gout.git
cd Gout
cargo build -p goutd -p gout
```

Rust SDK（在 `Cargo.toml` 中添加）：
```toml
gout-api = "0.3"
```

## 快速开始

### 服务端

```bash
# 在公网 VPS 上运行
goutd
```

输出：

```
2026-07-13T04:47:52.890Z  INFO  goutd: tunnel port range: 10000-32767 (22768 ports)
2026-07-13T04:47:52.891Z  INFO  goutd: HTTP server listening on http://127.0.0.1:8080
2026-07-13T04:47:52.891Z  INFO  goutd: Data server listening on 0.0.0.0:8081
```

首次启动还会生成管理 key：

```
──────────────────────────────────────────
  Admin API Key: sk-xxxxxxxxxxxx
  Save this key! It won't be shown again.
──────────────────────────────────────────
```

Web 面板默认只监听 `127.0.0.1`，通过 SSH 端口转发访问：

```bash
ssh -L 8080:localhost:8080 your-server
# 浏览器打开 http://localhost:8080
```

### 客户端

```bash
# 添加 server
$ gout server set prod prod.example.com:8080 sk-xxxxxxxxxxxx
[+] server "prod" (prod.example.com:8080) saved

# 前台运行
gout tcp 4000
[+] tcp tunnel: 127.0.0.1:4000 -> 127.0.0.1:10001
[+] signal channel established, waiting for connections...
    Ctrl+C to close tunnel

# 后台运行
gout tcp 8080 -d
[+] tcp tunnel: 127.0.0.1:8080 -> prod.example.com:10002
[+] tunnel started in background (PID: 87654)
    `gout ls` to check status
    `gout log 8080` to view logs
    `gout kill 8080` to stop

# 按 server 选择
gout tcp 8080 -s dev         # 用 dev server
gout tcp 8080 -s prod        # 用 prod server
gout tcp 8080 -r 10080       # 指定远程端口 10080

# 管理后台隧道
gout ls
  prod  prod.example.com:8080
    8080  prod.example.com:10002         tcp    87654

  dev  dev.example.com:8080
    (no active tunnels)

gout log 8080          # 查看日志
gout log 8080 -f       # 实时跟踪（Unix）

gout kill 8080
[+] tunnel on port 8080 (PID 87654) stopped

# 管理 server
gout server show
  prod  prod.example.com:8080
  dev   dev.example.com:8080  ← default

  `gout server default <name>` to change
  `gout server unset <name>` to remove

gout server default dev
gout server unset old-server

gout udp 53            # UDP 隧道（DNS 等）
gout http 8080         # HTTP 隧道，显示 http:// URL
```

## 配置文件

多 server 配置存储在 `~/.gout/config.toml`（旧 `~/.goutrc` 自动迁移）：

```toml
default_server = "prod"

[servers.prod]
addr = "prod.example.com:8080"
api_key = "sk-xxxxxxxxxxxx"

[servers.dev]
addr = "dev.example.com:8080"
api_key = "sk-yyyyyyyyyyyy"
```

后台隧道状态存储在 `~/.gout/daemon/`：

```
~/.gout/
├── config.toml       ← 多 server 配置
└── daemon/
    ├── 8080.json     ← PID 文件
    ├── 8080.log      ← 日志
    ├── 53.json
    └── 53.log
```

---

## REST API 参考

所有请求和响应均为 JSON 格式。

### 认证

两种 key 类型：

| Header | 类型 | 用途 |
|--------|------|------|
| `X-Admin-Key` | admin | 管理 API key（增删查） |
| `X-Api-Key` | tunnel | 创建/删除隧道 |

> 首次启动时自动生成 admin key，打印到 stdout。
> 通过 Web 面板或管理 API 创建普通 tunnel key。

### 管理 API

**创建 API key**

```http
POST /api/v1/keys
X-Admin-Key: <admin-key>
Content-Type: application/json

{"name": "我的笔记本"}
```

```json
{"success": true, "data": {"key": "sk-xxx...", "name": "我的笔记本"}}
```

**列出所有 key**

```http
GET /api/v1/keys
X-Admin-Key: <admin-key>
```

```json
{"success": true, "data": [{"key": "sk-xxx...", "name": "我的笔记本"}]}
```

**删除 key**

```http
DELETE /api/v1/keys/sk-xxx...
X-Admin-Key: <admin-key>
```

```json
{"success": true, "data": null}
```

### 隧道 API

**创建隧道**

```http
POST /api/v1/tunnels
X-Api-Key: <tunnel-key>
Content-Type: application/json

{"type": "tcp", "local_port": 4000, "remote_port": 10080}
```

```json
{
  "success": true,
  "data": {
    "token": 15735302723313469543,
    "public_port": 10000,
    "data_port": 8081,
    "tunnel_type": "tcp"
  }
}
```

创建后客户端需要连接数据端口（`data_port`）完成握手，详见"数据通道协议"。

**删除隧道**

```http
DELETE /api/v1/tunnels/15735302723313469543
X-Api-Key: <tunnel-key>
```

```json
{"success": true, "data": null}
```

**隧道类型**

| type | 说明 |
|------|------|
| `tcp` | TCP 隧道，每个外部连接一条独立数据通道 |
| `udp` | UDP 隧道，一条持久数据通道承载数据报帧，双向转发 |
| `http` | 等价于 `tcp`（URL 格式显示 `http://` 路径） |

---

## 数据通道协议

创建隧道后的数据通道握手流程：

```
TCP 隧道:
客户端                           服务端
  │                                │
  │── [token: u64 BE][type: u8] ──►│  握手（9 字节）
  │◄──── [status: u8] ─────────────│  0x01=OK, 0x00=Error
  │                                │
  │  （信号通道循环）               │
  │◄──── [0x02] ───────────────────│  新外部连接通知
  │                                │
  │  客户端另开连接：                │
  │── [token][type] ──────────────►│  数据通道握手
  │◄──── [0x01] ──────────────────│  OK
  │══════ raw bytes ══════════════│  双向 pipe

UDP 隧道:
客户端                           服务端
  │                                │
  │── [token: u64 BE][type: u8] ──►│  握手（9 字节）
  │◄──── [status: u8] ─────────────│  0x01=OK
  │══════ [len: u16][data] ═══════│  持久双向帧通道
  │        len=0 = 关闭信号       │
```

UDP 隧道使用帧格式：

```
[len: u16 BE][data: len bytes]
len = 0 表示关闭信号
```

---

## Rust SDK (gout-api)

`gout-api` 是 Rust crate，提供协议类型和客户端。

### 添加依赖

```toml
[dependencies]
gout-api = { git = "https://github.com/fb0sh/Gout" }
```

### GoutClient（隧道操作）

```rust
use gout_api::client::GoutClient;
use gout_api::TunnelType;

let gout = GoutClient::new("server.example.com:8080", "sk-xxxx...");

// 创建隧道
let tunnel = gout.create_tunnel(TunnelType::Tcp, 4000, None).await?;
println!("公网端口: {}", tunnel.public_port);

// 连接数据端口（握手由 data_channel 模块处理）
let mut stream = tokio::net::TcpStream::connect(
    format!("server.example.com:{}", tunnel.data_port)
).await?;

gout_api::data_channel::client_handshake(
    &mut stream, tunnel.token, TunnelType::Tcp
).await?;

// 连接本地服务
let local = tokio::net::TcpStream::connect("127.0.0.1:4000").await?;

// 双向 pipe
gout_api::data_channel::pipe_bidirectional(stream, local).await;

// 删除隧道
gout.delete_tunnel(tunnel.token).await?;
```

### GoutAdminClient（管理操作）

```rust
use gout_api::admin::GoutAdminClient;

let admin = GoutAdminClient::new("server.example.com:8080", "admin-key-xxx...");

// 创建 tunnel key
let key = admin.create_key("我的笔记本").await?;
println!("新 key: {}", key.key);

// 列出所有 key
let keys = admin.list_keys().await?;
for k in keys {
    println!("{} ({})", k.name, k.key);
}

// 删除 key
admin.delete_key("sk-xxx...").await?;
```

### 数据通道协议（底层）

```rust
use gout_api::data_channel;

// 客户端握手
data_channel::client_handshake(&mut stream, token, TunnelType::Tcp).await?;

// 服务端接收握手
let (token, tt) = data_channel::server_receive_handshake(&mut stream).await?;

// 服务端确认/拒绝
data_channel::server_accept(&mut stream).await?;
data_channel::server_reject(&mut stream, "reason").await?;

// 双向 pipe
data_channel::pipe_bidirectional(a, b).await;
```

---

## CLI 参考

### gout

```
轻量内网穿透工具

Usage: gout <COMMAND>

Commands:
  login  登录远程服务器（等价于 `server set`）
  tcp    创建 TCP 隧道
  udp    创建 UDP 隧道
  http   创建 HTTP 隧道（等价于 TCP）
  ls     列出本地后台隧道（按 server 分组）
  log    查看后台隧道日志
  kill   停止后台隧道
  server 管理 server（set / default / unset / show）
  help   Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

```
# 子命令帮助示例
$ gout tcp --help
创建 TCP 隧道

Usage: gout tcp [OPTIONS] <PORT>

Arguments:
  <PORT>  本地端口号

Options:
  -d, --detach                 后台运行
  -s, --server <SERVER>        服务器名称或地址（默认使用配置中的 default_server）
  -r, --remote-port <REMOTE_PORT>  远端公网端口（可选，不指定由服务端自动分配）
  -h, --help                  Print help

$ gout ls --help
列出本地后台隧道（按 server 分组）。
也支持 `gout list`。

Usage: gout list

Options:
  -h, --help  Print help

$ gout server --help
管理 server

Usage: gout server <COMMAND>

Commands:
  set     添加或更新 server
  default 设置默认 server
  unset   删除 server
  show    显示所有 server

$ gout server set --help
添加或更新 server

Usage: gout server set <NAME> <HOST> <KEY>

Arguments:
  <NAME>  server 名称
  <HOST>  server 地址，如 `server.example.com:8080`
  <KEY>   API key

Options:
  -h, --help  Print help

$ gout log --help
查看后台隧道日志

Usage: gout log [OPTIONS] <PORT>

Arguments:
  <PORT>  本地端口号

Options:
  -f, --follow  持续跟随（类似 tail -f）
  -h, --help    Print help

$ gout kill --help
停止后台隧道

Usage: gout kill <PORT>

Arguments:
  <PORT>  本地端口号

Options:
  -h, --help  Print help

$ gout server --help
管理 server

Usage: gout server <COMMAND>

Commands:
  set     添加或更新 server
  default 设置默认 server
  unset   删除 server
  show    显示所有 server

$ gout server set --help
添加或更新 server

Usage: gout server set <NAME> <HOST> <KEY>

Arguments:
  <NAME>  server 名称
  <HOST>  server 地址，如 `server.example.com:8080`
  <KEY>   API key

Options:
  -h, --help  Print help

```

### goutd

```
Gout 服务端守护进程

Usage: goutd [OPTIONS]

Options:
      --http-addr <HTTP_ADDR>    HTTP / Web 面板监听地址 [default: 127.0.0.1:8080]
      --data-addr <DATA_ADDR>    数据通道监听地址 [default: 0.0.0.0:8081]
      --port-start <PORT_START>  公网端口范围起始（默认 10000，避开系统临时端口范围） [default: 10000]
      --port-end <PORT_END>      公网端口范围结束（默认 32767，Linux 临时端口默认从 32768 开始） [default: 32767]
      --data-dir <DATA_DIR>      数据存储目录（keys.toml 存放位置） [default: ./gout-data]
  -h, --help                     Print help
  -V, --version                  Print version
```

## 开发

```bash
cargo build          # 编译全部
cargo test           # 运行所有测试
cargo run -p goutd   # 启动服务端
cargo run -p gout    # CLI 客户端
```

## License

MIT
