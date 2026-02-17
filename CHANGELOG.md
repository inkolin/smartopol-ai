# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-02-18

### Added

- Rust workspace with 3 crates: `skynet-core`, `skynet-protocol`, `skynet-gateway`
- Axum HTTP server on port 18789 with `/health` endpoint
- WebSocket handler with OpenClaw protocol v3 compatibility
- Handshake state machine: challenge → auth → hello-ok
- Authentication modes: token, password, none
- Heartbeat tick events every 30 seconds
- Handshake timeout (10s) and payload size enforcement (128KB)
- Event broadcast channel for connected clients
- Wire protocol types with full serialization/deserialization
- 8 wire compatibility tests
- Domain types: UserId (UUIDv7), AgentId, SessionKey, ConnId, UserRole
- Configuration via TOML file + SKYNET_* environment variable overrides
- SOUL.md agent persona file
- Project documentation: architecture, getting started, API reference
