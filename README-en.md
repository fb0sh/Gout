# Gout

A lightweight reverse proxy / tunnel tool inspired by frp and rathole. Expose your local service to the public internet with a single command.

## Features

- **Zero-config client**: `gout login server:8080 key` → `gout tcp 4000` — no config file to write
- **Web dashboard**: manage API keys, view active tunnels, add remarks — all from the browser
- **REST control plane**: tunnels are created and destroyed via HTTP API, not file-based config
- **Multi-protocol**: TCP, UDP, and HTTP (alias for TCP in v0.1)
- **Signal-channel architecture**: a lightweight control connection per tunnel notifies the client of new external connections; each connection gets its own dedicated data channel
- **Token-based auth**: each tunnel gets a random 64‑bit token for data‑plane authentication
- **Port range management**: pool-based allocation, no port fragmentation
- **Auto‑cleanup**: tunnels expire after 30 seconds if the client never connects the data channel; closing the control connection tears everything down

## Architecture

```
gout (CLI)  ──REST──►  goutd :8080 (HTTP api + dashboard)
     │                      │
     └──data channel──►  goutd :8081 (raw TCP)
```

Three crates:
- **gout-api** — Rust SDK: protocol types, `GoutClient` (tunnel ops), `GoutAdminClient` (management), `data_channel` (handshake/pipe)
- **gout** — CLI client: login, tcp, udp, http subcommands; reads `~/.goutrc`
- **goutd** — server daemon: axum HTTP server + tokio raw TCP data server

## Quick Start

### Server

```bash
# On your public VPS
goutd

# Output:
# ──────────────────────────────────────────
#   Initial admin key: sk-xxxxxxxxxxxx
#   Save this key! It won't be shown again.
# ──────────────────────────────────────────
# HTTP server listening on http://127.0.0.1:8080
# Data server listening on 0.0.0.0:8081
```

Dashboard listens on `127.0.0.1` only. Access via SSH port forwarding:

```bash
ssh -L 8080:localhost:8080 your-server
# Open http://localhost:8080 in your browser
```

### Client

```bash
# Create a tunnel API key via the web dashboard, then:
gout login server.example.com:8080 sk-xxxxxxxxxxxx
gout tcp 4000     # expose localhost:4000 to the public internet
```

---

## REST API Reference

All requests and responses are JSON.

### Authentication

Two key types:

| Header | Type | Purpose |
|--------|------|---------|
| `X-Admin-Key` | admin | Manage API keys (CRUD) |
| `X-Api-Key` | tunnel | Create / delete tunnels |

> The admin key is auto-generated on first startup and printed to stdout.
> Create tunnel keys via the web dashboard or the management API.

### Management API

**Create API key**

```http
POST /api/v1/keys
X-Admin-Key: <admin-key>
Content-Type: application/json

{"name": "my laptop"}
```

```json
{"success": true, "data": {"key": "sk-xxx...", "name": "my laptop"}}
```

**List all keys**

```http
GET /api/v1/keys
X-Admin-Key: <admin-key>
```

```json
{"success": true, "data": [{"key": "sk-xxx...", "name": "my laptop"}]}
```

**Delete a key**

```http
DELETE /api/v1/keys/sk-xxx...
X-Admin-Key: <admin-key>
```

```json
{"success": true, "data": null}
```

### Tunnel API

**Create a tunnel**

```http
POST /api/v1/tunnels
X-Api-Key: <tunnel-key>
Content-Type: application/json

{"type": "tcp", "local_port": 4000}
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

After creation the client must connect the data port and perform the handshake (see Data Channel Protocol).

**Delete a tunnel**

```http
DELETE /api/v1/tunnels/15735302723313469543
X-Api-Key: <tunnel-key>
```

```json
{"success": true, "data": null}
```

**Tunnel types**

| type | Description |
|------|-------------|
| `tcp` | TCP tunnel — one data channel per external connection |
| `udp` | UDP tunnel — one persistent data channel, framed datagrams |
| `http` | v0.1 alias for `tcp` |

---

## Data Channel Protocol

Handshake flow after tunnel creation:

```
Client                             Server
  │                                  │
  │── [token: u64 BE][type: u8] ───►│  handshake (9 bytes)
  │◄──── [status: u8] ──────────────│  0x01=OK, 0x00=Error
  │                                  │
  │  (TCP: signal channel loop)      │
  │◄──── [0x02] ────────────────────│  new external connection
  │                                  │
  │  Client opens a new connection:  │
  │── [token][type] ───────────────►│  data channel handshake
  │◄──── [0x01] ────────────────────│  OK
  │══════ raw bytes ═══════════════│  bidirectional pipe
```

UDP uses a framed format over the data channel:

```
[len: u16 BE][data: len bytes]
len = 0 signals close
```

---

## Rust SDK (gout-api)

Add the dependency:

```toml
[dependencies]
gout-api = { git = "https://github.com/fb0sh/Gout" }
```

### GoutClient (tunnel operations)

```rust
use gout_api::client::GoutClient;
use gout_api::TunnelType;

let gout = GoutClient::new("server.example.com:8080", "sk-xxxx...");

// Create a tunnel
let tunnel = gout.create_tunnel(TunnelType::Tcp, 4000).await?;
println!("public port: {}", tunnel.public_port);

// Connect the data port (handshake via data_channel module)
let mut stream = tokio::net::TcpStream::connect(
    format!("server.example.com:{}", tunnel.data_port)
).await?;

gout_api::data_channel::client_handshake(
    &mut stream, tunnel.token, TunnelType::Tcp
).await?;

// Connect local service
let local = tokio::net::TcpStream::connect("127.0.0.1:4000").await?;

// Bidirectional pipe
gout_api::data_channel::pipe_bidirectional(stream, local).await;

// Delete the tunnel
gout.delete_tunnel(tunnel.token).await?;
```

### GoutAdminClient (management)

```rust
use gout_api::admin::GoutAdminClient;

let admin = GoutAdminClient::new("server.example.com:8080", "admin-key-xxx...");

// Create a tunnel key
let key = admin.create_key("my laptop").await?;
println!("new key: {}", key.key);

// List all keys
let keys = admin.list_keys().await?;
for k in &keys {
    println!("{} ({})", k.name, k.key);
}

// Delete a key
admin.delete_key("sk-xxx...").await?;
```

### Low-level data channel protocol

```rust
use gout_api::data_channel;

// Client handshake
data_channel::client_handshake(&mut stream, token, TunnelType::Tcp).await?;

// Server receive handshake
let (token, tt) = data_channel::server_receive_handshake(&mut stream).await?;

// Server accept / reject
data_channel::server_accept(&mut stream).await?;
data_channel::server_reject(&mut stream, "reason").await?;

// Bidirectional pipe
data_channel::pipe_bidirectional(a, b).await;
```

---

## Development

```bash
cargo build          # build everything
cargo test           # run all tests
cargo run -p goutd   # start the server
cargo run -p gout    # CLI client
```

## License

MIT
