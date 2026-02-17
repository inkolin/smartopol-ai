# API Reference

## HTTP Endpoints

### GET /health

Liveness probe. Returns server metadata.

```json
{
  "status": "ok",
  "version": "0.1.0",
  "protocol": 3,
  "ws_clients": 0
}
```

### GET /ws

WebSocket upgrade endpoint. All client interaction happens over this connection.

## WebSocket Protocol

### Frame Types

All frames are JSON objects with a `type` discriminator.

#### REQ (client → server)
```json
{ "type": "req", "id": "unique-id", "method": "chat.send", "params": {} }
```

#### RES (server → client)
```json
{ "type": "res", "id": "unique-id", "ok": true, "payload": {} }
```

#### EVENT (server → client, unsolicited)
```json
{ "type": "event", "event": "tick", "payload": {}, "seq": 42 }
```

## Methods

| Method | Description | Status |
|---|---|---|
| `connect` | Handshake authentication | Implemented |
| `ping` | Liveness check | Implemented |
| `agent.status` | Agent runtime status | Stub |
| `chat.send` | Send message to agent | Phase 2 |
| `sessions.list` | List user sessions | Phase 3 |

## Limits

- Max payload: 128 KB
- Slow consumer threshold: 1 MB buffered
- Handshake timeout: 10 seconds
- Heartbeat interval: 30 seconds
