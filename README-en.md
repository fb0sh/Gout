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

The project is a Cargo workspace with three crates:
- **gout-proto** — shared protocol types, wire format encoding, REST API types
- **gout** — CLI client: login, tcp, udp, http subcommands; reads `~/.goutrc`
- **goutd** — server daemon: axum HTTP server + tokio raw TCP data server

## Quick Start

```bash
# server (public VPS)
goutd
# → Initial API key: sk-xxxxxxxxxxxx
# → Dashboard at http://127.0.0.1:8080 (SSH tunnel: ssh -L 8080:localhost:8080 server)

# client
gout login server.example.com:8080 sk-xxxxxxxxxxxx
gout tcp 4000     # expose localhost:4000 to the public internet
```

## License

MIT
