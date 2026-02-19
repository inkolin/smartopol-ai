#!/usr/bin/env bash
# SmartopolAI Docker Setup — one-command deployment
# Usage: ./docker-setup.sh
set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

info()  { echo -e "${CYAN}[INFO]${NC}  $*"; }
ok()    { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC}  $*"; }
fail()  { echo -e "${RED}[FAIL]${NC}  $*"; exit 1; }

# ── Prerequisites ─────────────────────────────────────────────────────────────
command -v docker >/dev/null 2>&1 || fail "Docker not found. Install: https://docs.docker.com/get-docker/"

if docker compose version >/dev/null 2>&1; then
    COMPOSE="docker compose"
elif command -v docker-compose >/dev/null 2>&1; then
    COMPOSE="docker-compose"
else
    fail "Docker Compose not found. Install: https://docs.docker.com/compose/install/"
fi
ok "Docker and Compose detected"

# ── Generate .env if missing ──────────────────────────────────────────────────
if [ ! -f .env ]; then
    info "Generating .env file..."

    AUTH_TOKEN=$(openssl rand -hex 32 2>/dev/null || head -c 64 /dev/urandom | od -An -tx1 | tr -d ' \n')

    cat > .env <<EOF
# SmartopolAI Docker configuration
# Generated on $(date -u +%Y-%m-%dT%H:%M:%SZ)

# Gateway auth token (required for API access)
SKYNET_AUTH_TOKEN=${AUTH_TOKEN}

# Port mapping (default: 18789)
# SKYNET_PORT=18789

# LLM provider — uncomment ONE and set your API key:
# ANTHROPIC_API_KEY=sk-ant-...
# OPENAI_API_KEY=sk-...

# Agent model (default: claude-sonnet-4-6)
# SKYNET_MODEL=claude-sonnet-4-6

# Discord bot (optional)
# DISCORD_BOT_TOKEN=

# Ollama (local LLM) — also uncomment extra_hosts in docker-compose.yml
# SKYNET_PROVIDERS_OLLAMA_BASE_URL=http://host.docker.internal:11434
EOF

    chmod 600 .env
    ok "Created .env (auth token generated, API keys need manual setup)"
    warn "Edit .env to add your LLM provider API key before first use"
else
    ok "Using existing .env"
fi

# ── Build ─────────────────────────────────────────────────────────────────────
info "Building Docker image (this may take a few minutes on first run)..."
$COMPOSE build || fail "Docker build failed"
ok "Image built"

# ── Start ─────────────────────────────────────────────────────────────────────
info "Starting SmartopolAI..."
$COMPOSE up -d || fail "Failed to start container"
ok "Container started"

# ── Health check ──────────────────────────────────────────────────────────────
PORT=$(grep -E '^SKYNET_PORT=' .env 2>/dev/null | cut -d= -f2 || echo "18789")
PORT=${PORT:-18789}

info "Waiting for health check..."
for i in $(seq 1 15); do
    if curl -sf "http://localhost:${PORT}/health" >/dev/null 2>&1; then
        ok "SmartopolAI is running!"
        echo ""
        echo -e "${GREEN}═══════════════════════════════════════════════════${NC}"
        echo -e "${GREEN}  SmartopolAI is ready!${NC}"
        echo -e "${GREEN}═══════════════════════════════════════════════════${NC}"
        echo ""
        echo -e "  Health:  http://localhost:${PORT}/health"
        echo -e "  API:     ws://localhost:${PORT}/ws"
        echo ""
        echo -e "  ${CYAN}Useful commands:${NC}"
        echo -e "    Logs:    $COMPOSE logs -f"
        echo -e "    Stop:    $COMPOSE down"
        echo -e "    Update:  git pull && $COMPOSE build && $COMPOSE up -d"
        echo ""
        exit 0
    fi
    sleep 1
done

warn "Health check timed out — container may still be starting."
echo "  Check logs:   $COMPOSE logs -f"
echo "  Check status: $COMPOSE ps"
exit 1
