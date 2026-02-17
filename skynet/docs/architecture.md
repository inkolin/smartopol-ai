# Skynet Architecture

Skynet is the Rust core of SmartopolAI — a multi-channel AI gateway that replaces OpenClaw (Node.js).

## Workspace Structure

```
skynet/
  crates/
    skynet-core/       # shared types, config, errors
    skynet-protocol/   # OpenClaw-compatible wire frame types
    skynet-gateway/    # Axum HTTP/WS server (main binary)
```

## Design Principles

1. **Protocol compatible** — existing OpenClaw CLI clients connect without changes
2. **User-centric sessions** — sessions belong to users, not channels
3. **SQLite-only** — zero external dependencies, single binary deployment
4. **Explicit over abstract** — no premature abstraction, readable for contributors

## Wire Protocol

Skynet implements OpenClaw protocol v3 over WebSocket:

- `REQ` — client request: `{ type: "req", id, method, params? }`
- `RES` — server response: `{ type: "res", id, ok, payload?, error? }`
- `EVENT` — server push: `{ type: "event", event, payload?, seq? }`

### Handshake Sequence

1. Server sends `EVENT connect.challenge { nonce }`
2. Client sends `REQ connect { auth: { mode, ... } }`
3. Server sends `RES hello-ok { protocol: 3, features, ... }`

## Auth Modes

- `token` — bearer token comparison (default)
- `password` — plaintext now, argon2id later
- `none` — open access (dev only)
- `tailscale`, `device-token`, `trusted-proxy` — planned

## Configuration

Config loaded from `~/.skynet/skynet.toml` with `SKYNET_*` env overrides.
Default port: 18789.
