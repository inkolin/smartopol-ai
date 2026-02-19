# ─── Stage 1: Chef ────────────────────────────────────────────────────────────
# Install cargo-chef for dependency caching across 12 workspace crates.
FROM rust:1.83-slim AS chef

RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/* \
    && cargo install cargo-chef --locked

WORKDIR /build

# ─── Stage 2: Planner ────────────────────────────────────────────────────────
# Analyze workspace and produce a dependency recipe (no source code needed).
FROM chef AS planner

COPY skynet/ skynet/
COPY vendor/ vendor/

WORKDIR /build/skynet
RUN cargo chef prepare --recipe-path /build/recipe.json

# ─── Stage 3: Dependencies ───────────────────────────────────────────────────
# Cook (build) only the dependencies from the recipe — cached as a Docker layer.
FROM chef AS deps

COPY --from=planner /build/recipe.json /build/recipe.json
COPY vendor/ vendor/

WORKDIR /build/skynet
RUN cargo chef cook --release --recipe-path /build/recipe.json

# ─── Stage 4: Builder ────────────────────────────────────────────────────────
# Copy real source and build the final binary.
FROM deps AS builder

COPY skynet/ skynet/
COPY vendor/ vendor/

WORKDIR /build/skynet
RUN cargo build --release --bin skynet-gateway \
    && strip /build/skynet/target/release/skynet-gateway

# ─── Stage 5a: Development runtime ──────────────────────────────────────────
# Slim Debian with curl for health checks and debugging.
FROM debian:bookworm-slim AS dev

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

# Non-root user
RUN groupadd -g 10001 skynet \
    && useradd -u 10001 -g skynet -m -s /bin/sh skynet

# Binary
COPY --from=builder /build/skynet/target/release/skynet-gateway /usr/local/bin/skynet-gateway

# Default config templates (available if user bind-mounts /etc/skynet/)
COPY skynet/config/default.toml    /etc/skynet/default.toml
COPY skynet/config/SOUL.template.md /etc/skynet/SOUL.template.md

# Data directory — mount a named volume here
RUN mkdir -p /home/skynet/.skynet && chown -R skynet:skynet /home/skynet/.skynet

USER skynet
WORKDIR /home/skynet

# Figment reads SKYNET_* env vars — bind to 0.0.0.0 so the port is reachable.
ENV SKYNET_GATEWAY_BIND=0.0.0.0
ENV SKYNET_GATEWAY_PORT=18789
ENV HOME=/home/skynet

EXPOSE 18789

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -sf http://localhost:18789/health || exit 1

ENTRYPOINT ["skynet-gateway"]

# ─── Stage 5b: Production runtime (distroless) ──────────────────────────────
# Minimal image — no shell, no package manager, ~25 MB.
FROM gcr.io/distroless/cc-debian12:nonroot AS release

COPY --from=builder /build/skynet/target/release/skynet-gateway /usr/local/bin/skynet-gateway
COPY skynet/config/default.toml    /etc/skynet/default.toml
COPY skynet/config/SOUL.template.md /etc/skynet/SOUL.template.md

ENV SKYNET_GATEWAY_BIND=0.0.0.0
ENV SKYNET_GATEWAY_PORT=18789
ENV HOME=/home/nonroot

EXPOSE 18789

# No HEALTHCHECK in distroless (no curl). Use docker-compose healthcheck instead.
ENTRYPOINT ["skynet-gateway"]
