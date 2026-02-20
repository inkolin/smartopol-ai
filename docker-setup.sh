#!/usr/bin/env bash
# SmartopolAI Docker Setup — one-command deployment
# Usage: ./docker-setup.sh
set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
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

    # ── Provider selection ────────────────────────────────────────────────────
    echo ""
    echo -e "${BOLD}Which LLM provider will you use?${NC}"
    echo "  1) Anthropic (Claude)"
    echo "  2) OpenAI (GPT)"
    echo "  3) Groq"
    echo "  4) DeepSeek"
    echo "  5) OpenRouter"
    echo "  6) Google (Gemini)"
    echo "  7) Ollama (local)"
    echo "  8) Other / I'll configure later"
    echo ""
    read -rp "Select [1-8]: " provider_choice

    PROVIDER_LINE=""
    PROVIDER_NAME=""
    case "${provider_choice}" in
        1)
            read -rp "Enter your Anthropic API key: " api_key
            PROVIDER_LINE="ANTHROPIC_API_KEY=${api_key}"
            PROVIDER_NAME="Anthropic"
            ;;
        2)
            read -rp "Enter your OpenAI API key: " api_key
            PROVIDER_LINE="OPENAI_API_KEY=${api_key}"
            PROVIDER_NAME="OpenAI"
            ;;
        3)
            read -rp "Enter your Groq API key: " api_key
            PROVIDER_LINE="GROQ_API_KEY=${api_key}"
            PROVIDER_NAME="Groq"
            ;;
        4)
            read -rp "Enter your DeepSeek API key: " api_key
            PROVIDER_LINE="DEEPSEEK_API_KEY=${api_key}"
            PROVIDER_NAME="DeepSeek"
            ;;
        5)
            read -rp "Enter your OpenRouter API key: " api_key
            PROVIDER_LINE="OPENROUTER_API_KEY=${api_key}"
            PROVIDER_NAME="OpenRouter"
            ;;
        6)
            read -rp "Enter your Google API key: " api_key
            PROVIDER_LINE="GOOGLE_API_KEY=${api_key}"
            PROVIDER_NAME="Google"
            ;;
        7)
            PROVIDER_LINE="SKYNET_PROVIDERS_OLLAMA_BASE_URL=http://host.docker.internal:11434"
            PROVIDER_NAME="Ollama"
            warn "Remember to uncomment extra_hosts in docker-compose.yml for Ollama"
            ;;
        *)
            PROVIDER_NAME=""
            ;;
    esac

    cat > .env <<EOF
# SmartopolAI Docker configuration
# Generated on $(date -u +%Y-%m-%dT%H:%M:%SZ)

# Gateway auth token (required for API access)
SKYNET_AUTH_TOKEN=${AUTH_TOKEN}

# Port mapping (default: 18789)
# SKYNET_PORT=18789

# LLM provider API keys (uncomment and set the ones you use)
${PROVIDER_LINE:-# ANTHROPIC_API_KEY=sk-ant-...}
# OPENAI_API_KEY=sk-...
# GROQ_API_KEY=gsk_...
# DEEPSEEK_API_KEY=sk-...
# OPENROUTER_API_KEY=sk-or-...
# GOOGLE_API_KEY=...

# Agent model (default: claude-sonnet-4-6)
# SKYNET_MODEL=claude-sonnet-4-6

# Discord bot (optional)
# DISCORD_BOT_TOKEN=

# Ollama (local LLM) — also uncomment extra_hosts in docker-compose.yml
# SKYNET_PROVIDERS_OLLAMA_BASE_URL=http://host.docker.internal:11434
EOF

    chmod 600 .env
    if [ -n "${PROVIDER_NAME}" ]; then
        ok "Created .env with ${PROVIDER_NAME} provider configured"
    else
        ok "Created .env (auth token generated, API keys need manual setup)"
        warn "Edit .env to add your LLM provider API key before first use"
    fi
else
    ok "Using existing .env"
fi

# ── Pull or Build ────────────────────────────────────────────────────────────
# Check if docker-compose.yml uses a pre-built image or build-from-source
if grep -q '^\s*image:' docker-compose.yml && ! grep -q '^\s*build:' docker-compose.yml; then
    info "Pulling pre-built image..."
    $COMPOSE pull || fail "Failed to pull image"
    ok "Image pulled"
else
    info "Building Docker image (this may take a few minutes on first run)..."
    $COMPOSE build || fail "Docker build failed"
    ok "Image built"
fi

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
        echo -e "    Update:  $COMPOSE pull && $COMPOSE up -d"
        echo ""
        exit 0
    fi
    sleep 1
done

warn "Health check timed out — container may still be starting."
echo -e "  Check logs:   ${CYAN}$COMPOSE logs -f${NC}"
echo -e "  Check status: ${CYAN}$COMPOSE ps${NC}"
echo ""
echo "  If the container keeps restarting, check that your API key is valid."
exit 1
