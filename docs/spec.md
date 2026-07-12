# Spec: Gout v0.1 — 轻量内网穿透工具

## Problem Statement

开发者想把本地服务暴露到公网（调试 webhook、演示 demo、远程访问本地数据库），但现有工具（frp、rathole）需要写配置文件才能启动，没有内置的管理面板，配置繁琐，上手成本高。

## Solution

Gout 是一个 C/S 架构的内网穿透工具。服务端 `goutd` 部署在公网 VPS 上，自带 Web 面板管理 API key 和查看隧道状态。客户端 `gout` 通过两行命令即可将本地端口暴露到公网：

```bash
gout login server:8080 sk-xxx    # 只需一次
gout tcp 4000                    # 即时开启隧道
```

## User Stories

1. As a developer, I want to expose my local dev server to the public internet with one command, so that I can test webhooks without deploying.
2. As a developer, I want to save my server credentials once and reuse them across sessions, so that I don't have to type the address every time.
3. As a server admin, I want a web dashboard to create and manage API keys, so that I know who has access to my server.
4. As a server admin, I want to add a remark to each API key (e.g. "张三的笔记本"), so that I can identify which key belongs to which person.
5. As a server admin, I want to view all active tunnels on the dashboard, so that I know which ports are in use.
6. As a developer, I want to create a TCP tunnel to expose my local web server on port 3000, so that external users can access it.
7. As a developer, I want to create a UDP tunnel to expose my local DNS server on port 53, so that I can test DNS resolution from the public internet.
8. As a developer, I want to expose multiple local ports simultaneously (e.g. 3000 and 4000), so that I can run multiple services at once.
9. As a developer, I want the tunnel to close automatically when I press Ctrl+C, so that I don't leave open ports on the server.
10. As a server admin, I want the server to automatically clean up tunnels when a client disconnects, so that ports don't leak.
11. As a server admin, I want unused tunnels to time out if the client never connects, so that abandoned port reservations are reclaimed.
12. As a server admin, I want the web dashboard to only be accessible from localhost, so that it's not exposed to the public internet.
13. As a server admin, I want an initial API key to be auto-generated on first startup, so that the server is ready to use immediately.
14. As a developer, I want to see a clear error message if my local service isn't running when an external user connects, so that I know what to fix.

## Implementation Decisions

### Architecture

- **Three Rust crates** in a Cargo workspace: `gout-proto` (shared types), `gout` (CLI client), `goutd` (server daemon).
- **Control plane** is HTTP REST API — auth via `X-Api-Key` header, tunnel CRUD.
- **Data plane** is raw TCP on a separate port — authentication via a random `u64` token generated per-tunnel.
- **Web dashboard** is server-rendered HTML via `askama` templates, served by the same axum instance as the REST API.

### Tunnel lifecycle (TCP)

1. Client POSTs to `/api/v1/tunnels` with `{ type: "tcp", local_port: N }` → server allocates a public port and a random token, returns `{ token, public_port, data_port }`.
2. Client opens a TCP connection to `data_port`, sends `[token: u64 BE]` as handshake.
3. Server validates token, transitions tunnel state from `WaitingForHandshake` to `Active`.
4. When an external user connects to `public_port`:
   - Server accepts the TCP connection.
   - Server spawns a task: accept → notify client (details below) → bidirectional pipe.
5. TCP connection close (either side) triggers cleanup: server releases the public port, removes the tunnel record.

### Tunnel lifecycle (UDP)

Same as TCP except:
- Only one persistent data channel per UDP tunnel (not one per external connection).
- UDP datagrams are framed as `[len: u16 BE][data]` over the data channel.
- `len = 0` signals tunnel close.

### HTTP tunnel

`gout http` is an alias for `gout tcp` in v0.1. No HTTP-specific processing.

### Tunnel notification (TCP)

When a new external TCP connection arrives, the server needs to tell the client to open a data channel. Two options were considered:
- **Push notification**: server opens a connection *to* the client or uses a persistent signaling channel.
- **Pull notification**: the client maintains one persistent "notification channel" per tunnel, and the server sends control messages over it.

**Decision**: Use one persistent data channel per tunnel for notifications + data. When the client first connects (handshake), the connection becomes a bidirectional control+data channel. The server sends control bytes to signal new external connections. The client responds by opening a *second* data channel to pipe the actual external traffic. This avoids the complexity of server-initiated connections or separate signaling channels.

The state machine (from prototype):

```
Tunnel states: WaitingForHandshake → Active → Closed
Connection states (TCP): WaitingForClientData → Active → Closed
```

Key transitions encoded in prototype `TunnelManager::step(Action) -> Vec<Event>`:
- `CreateTunnel` → port allocated, tunnel in WaitingForHandshake, 30-tick timeout starts.
- `ClientHandshake` → tunnel becomes Active, timeout cleared.
- `ExternalConnect` → new connection created (WaitingForClientData), notification event emitted.
- `ClientDataConnect` → connection transitions to Active.
- `CloseTunnel` → all connections closed, port released.
- `Tick` → decrements handshake timeout for waiting tunnels; expired tunnels are closed.

**UDP correction**: The prototype currently treats UDP the same as TCP (creates individual conn_ids per external connection). The production implementation must differ: UDP tunnels use a single persistent data channel with no per-connection tracking. External datagrams are forwarded as `[len: u16][data]` frames; client responses are likewise framed.

### Port allocation

A free-port pool (`Vec<u16>`). On allocation: `pop()`. On release: `push()`. No fragmentation.

### Persistence

- **Client config** (`~/.goutrc`): TOML file with `server.addr` and `api_key`.
- **Server keys** (`{data_dir}/keys.toml`): TOML array of `{ key, name, created_at }` entries. Auto-generated initial key on first startup.
- **Tunnel state**: In-memory only. Lost on restart.

### Security posture (v0.1)

- REST API and Web dashboard bind to `127.0.0.1` only. Access via SSH port forwarding.
- Data channel is plain TCP with token-based authentication. TLS deferred to post-v0.1.
- API key auto-generated at first startup and printed to stdout.

### API contract

| Method | Path | Auth | Request | Response |
|--------|------|------|---------|----------|
| GET | `/dashboard` | — | — | HTML page |
| GET | `/keys` | — | — | HTML page |
| POST | `/api/v1/keys` | — | `{ name }` | `{ key, name }` |
| GET | `/api/v1/keys` | — | — | `[{ key, name }]` |
| DELETE | `/api/v1/keys/:key` | — | — | `{ success }` |
| POST | `/api/v1/tunnels` | X-Api-Key | `{ type, local_port? }` | `{ token, public_port, data_port, tunnel_type }` |
| DELETE | `/api/v1/tunnels/:token` | X-Api-Key | — | `{ success }` |

### Wire protocol (data channel)

```
Handshake (client → server, 9 bytes):
  [token: u64 BE]
  [tunnel_type: u8]   0=Tcp, 1=Udp

Response (server → client, 1 byte):
  [status: u8]  0x01=OK, 0x00=Error + [err_len: u16][err_msg]

TCP data channel (after handshake):
  raw bytes, bidirectional pipe. One connection per external connection.

UDP data channel (after handshake):
  [len: u16 BE][data: len bytes]   bidirectional framed datagrams
  len=0 = tunnel close signal
```

### Missing pieces to resolve

1. **TCP notification mechanism**: Exactly how the server tells the client "a new external connection arrived" over the existing data channel. Options: (a) repurpose the handshake connection as a notification channel — server sends a control byte, client opens a new TCP connection for the actual data; (b) use a separate notification channel. Decision pending — this is the one thing the prototype state machine abstracts away (it only tracks "notification emitted" as an event).

2. **Axiom**: Current setup assumes both REST API and Web dashboard bind to the **same port** via axum. But the data channel needs a **separate port**. The server binary must listen on two ports simultaneously: one for HTTP (axum), one for raw TCP (tokio `TcpListener`).

## Testing Decisions

### What makes a good test

Tests exercise external behavior — given an input, verify the output or state change. They do not test internal helper functions, logging, or implementation details. The highest-value seam is the one furthest from I/O.

### Test seams (highest to lowest)

1. **TunnelManager state machine** (`goutd::tunnel::TunnelManager`). Pure interface: `step(Action) -> Vec<Event>`. No I/O, no async, no side effects. This is the ideal seam — all tunnel lifecycle logic can be verified by driving actions and asserting events and state.

2. **Frame encoding/decoding** (`gout-proto`). Pure functions: `encode_handshake(token) -> [u8; 8]`, `decode_handshake(buf) -> u64`, `encode_frame(len, data) -> Vec<u8>`. Test round-trip: encode then decode yields original data.

3. **REST API handlers** (`goutd::api`). Use `axum::test` to send HTTP requests and assert status codes + JSON bodies. Mock the TunnelManager dependency.

4. **Key store** (`goutd::store`). Test with temp files: write keys, read back, verify contents. Test idempotency of auto-key generation.

5. **~/.goutrc read/write** (`gout::config`). Same pattern — temp file, write config, read back.

### Prior art

The repo has no existing tests (greenfield project). The prototype binary (`goutd/src/bin/prototype.rs`) serves as an interactive test harness for seam #1 — it demonstrates that the state machine is driveable through a pure interface.

## Out of Scope

- **TLS encryption** on the data channel. Will be added post-v0.1 via `tokio-rustls`.
- **HTTP Host-based routing** (one public port serving multiple domains). v0.1 HTTP is a TCP alias.
- **Connection pooling or keepalive**. Each external connection gets a fresh data channel.
- **Client-side daemon mode**. One process per tunnel in v0.1.
- **IPv6 support**.
- **Windows support** (not blocked, just not tested).
- **Rate limiting or DDoS protection**.
- **Audit logging** of tunnel creation/deletion beyond tracing.

## Further Notes

- The project is a Cargo workspace at `/Users/fb0sh/Projects/Gout/gout/`.
- Phase 1 (workspace skeleton + proto crate + prototype state machine) is complete.
- Phase 2 (goutd server) is next — this is where the notification mechanism needs to be nailed down.
- The prototype lives at `goutd/src/bin/prototype.rs` and the state machine at `goutd/src/tunnel.rs`. These will be cleaned up as the production implementation stabilizes.
