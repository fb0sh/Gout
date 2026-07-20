# Changelog

## 0.7.0 (2026-07-20)

### Added

- **`-r`/`--remote-port` flag**: `gout tcp`, `gout udp`, `gout http` now accept `-r <port>` to request a specific remote public port on the server.
  - When specified, the server directly binds the requested port (strict mode — no fallback if occupied).
  - When not specified, the server uses the existing PortAllocator auto-selection as before.
  - `CreateTunnelRequest` gains a `remote_port` field (`Option<u16>`).

## 0.6.0 (2026-07-13)

- Initial public release with PortAllocator, multi-server config, daemon mode, Web admin panel.
