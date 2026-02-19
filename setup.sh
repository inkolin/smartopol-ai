#!/usr/bin/env bash
# setup.sh — SmartopolAI interactive installer
# Usage: ./setup.sh
# Supports: Linux (x86_64 / aarch64) and macOS (x86_64 / Apple Silicon)

set -euo pipefail

# ─── Constants ────────────────────────────────────────────────────────────────
VERSION="0.2.0"
MIN_RUST_MINOR=80          # requires rustc 1.80+
SKYNET_DIR="$HOME/.skynet"
BINARY_NAME="skynet-gateway"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SOUL_TEMPLATE="$SCRIPT_DIR/skynet/config/SOUL.template.md"
SOUL_DEST="$SKYNET_DIR/SOUL.md"
CONFIG_DEST="$SKYNET_DIR/skynet.toml"
LOG_FILE="$SKYNET_DIR/skynet.log"
BUILD_LOG="/tmp/skynet-build.log"

# ─── Colours ──────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
RESET='\033[0m'

# ─── Global wizard output (set by wizard(), consumed by write_config / later steps)
GATEWAY_PORT="18789"
AUTH_TOKEN=""
PROVIDER_NAME=""
AGENT_MODEL=""
PROVIDER_TOML=""
DISCORD_TOML=""

# ─── Helpers ──────────────────────────────────────────────────────────────────
info()    { echo -e "${CYAN}  →${RESET} $*"; }
success() { echo -e "${GREEN}  ✓${RESET} $*"; }
warn()    { echo -e "${YELLOW}  !${RESET} $*"; }
die()     { echo -e "${RED}  ✗${RESET} $*" >&2; exit 1; }

# prompt VAR_NAME "Label" "default"
prompt() {
    local var_name="$1" label="$2" default="$3"
    local input
    if [[ -n "$default" ]]; then
        echo -ne "${BOLD}  ${label}${RESET} [${default}]: "
    else
        echo -ne "${BOLD}  ${label}${RESET}: "
    fi
    read -r input
    printf -v "$var_name" '%s' "${input:-$default}"
}

# prompt_secret VAR_NAME "Label"
prompt_secret() {
    local var_name="$1" label="$2"
    local input
    echo -ne "${BOLD}  ${label}${RESET}: "
    read -rs input
    echo
    printf -v "$var_name" '%s' "$input"
}

generate_token() {
    if command -v openssl &>/dev/null; then
        openssl rand -hex 32
    else
        head -c 24 /dev/urandom | base64 | tr -d '+/=' | head -c 32
    fi
}

# validate_api_key PROVIDER KEY [BASE_URL]
# Returns 0 if the key/server is valid.
validate_api_key() {
    local provider="$1" key="$2" base_url="${3:-}"
    local status
    info "Validating ${provider} connection..."
    case "$provider" in
        anthropic)
            status=$(curl -s -o /dev/null -w "%{http_code}" \
                --max-time 10 \
                -H "x-api-key: $key" \
                -H "anthropic-version: 2023-06-01" \
                https://api.anthropic.com/v1/models)
            [[ "$status" == "200" ]]
            ;;
        openai)
            status=$(curl -s -o /dev/null -w "%{http_code}" \
                --max-time 10 \
                -H "Authorization: Bearer $key" \
                https://api.openai.com/v1/models)
            [[ "$status" == "200" ]]
            ;;
        ollama)
            status=$(curl -s -o /dev/null -w "%{http_code}" \
                --max-time 5 \
                "${base_url}/api/tags")
            [[ "$status" == "200" ]]
            ;;
    esac
}

# Step 1 of wizard extracted so /setup-model can call it standalone.
# Loops until a valid provider+key is confirmed.
wizard_provider() {
    while true; do
        echo -e "${BOLD}Step 1 — AI Provider${RESET}"
        echo -e "    1) Anthropic Claude ${CYAN}(recommended)${RESET}"
        echo    "    2) OpenAI"
        echo    "    3) Ollama (local, free — runs on your machine)"
        echo
        local provider_choice=""
        prompt provider_choice "Choice" "1"
        echo

        local api_key="" ollama_url="" done=false

        case "$provider_choice" in
            2)
                PROVIDER_NAME="openai"
                AGENT_MODEL="gpt-4o"
                while true; do
                    prompt_secret api_key "OpenAI API key (sk-...)"
                    if [[ ! "$api_key" =~ ^sk- ]]; then
                        warn "OpenAI keys start with 'sk-'. Try again, or type 'back' to choose a different provider."
                        local cmd; read -r cmd
                        [[ "$cmd" == "back" ]] && break
                        continue
                    fi
                    if validate_api_key "openai" "$api_key"; then
                        success "OpenAI API key accepted"
                        PROVIDER_TOML="[providers.openai]
api_key = \"${api_key}\""
                        done=true
                        break
                    else
                        echo
                        warn "OpenAI rejected this key (wrong key or billing issue)."
                        warn "Get yours at: https://platform.openai.com/api-keys"
                        echo -ne "  Press Enter to retry, or type ${CYAN}back${RESET} to choose a different provider: "
                        local cmd; read -r cmd
                        [[ "$cmd" == "back" ]] && break
                    fi
                done
                ;;
            3)
                PROVIDER_NAME="ollama"
                AGENT_MODEL="llama3.2"
                prompt ollama_url "Ollama base URL" "http://localhost:11434"
                if validate_api_key "ollama" "" "$ollama_url"; then
                    success "Ollama server reachable at ${ollama_url}"
                else
                    warn "Ollama not reachable at ${ollama_url}."
                    warn "Start it with: ollama serve    →   https://ollama.com"
                    warn "Continuing — fix the URL in ${CONFIG_DEST} when ready."
                fi
                PROVIDER_TOML="[providers.ollama]
base_url = \"${ollama_url}\""
                done=true
                ;;
            *)
                PROVIDER_NAME="anthropic"
                AGENT_MODEL="claude-sonnet-4-6"
                while true; do
                    prompt_secret api_key "Anthropic API key (sk-ant-...)"
                    if [[ ! "$api_key" =~ ^sk-ant- ]]; then
                        warn "Anthropic keys start with 'sk-ant-'. Try again, or type 'back' to choose a different provider."
                        local cmd; read -r cmd
                        [[ "$cmd" == "back" ]] && break
                        continue
                    fi
                    if validate_api_key "anthropic" "$api_key"; then
                        success "Anthropic API key accepted"
                        PROVIDER_TOML="[providers.anthropic]
api_key = \"${api_key}\""
                        done=true
                        break
                    else
                        echo
                        warn "Anthropic rejected this key."
                        warn "Get yours at: https://console.anthropic.com"
                        echo -ne "  Press Enter to retry, or type ${CYAN}back${RESET} to choose a different provider: "
                        local cmd; read -r cmd
                        [[ "$cmd" == "back" ]] && break
                    fi
                done
                ;;
        esac

        $done && break
    done

    success "Provider: ${BOLD}${PROVIDER_NAME}${RESET} / model: ${BOLD}${AGENT_MODEL}${RESET}"
    echo
}

version_gte() {
    # Returns 0 if first version >= second (major.minor comparison)
    local have_major have_minor need_minor
    have_major=$(echo "$1" | cut -d. -f1)
    have_minor=$(echo "$1" | cut -d. -f2)
    need_minor=$(echo "$2" | cut -d. -f2)
    [[ "$have_major" -gt 1 ]] && return 0
    [[ "$have_major" -eq 1 && "$have_minor" -ge "$need_minor" ]] && return 0
    return 1
}

# ─── 1. Banner ────────────────────────────────────────────────────────────────
print_banner() {
    echo
    echo -e "${CYAN}  ╔═══════════════════════════════════════════╗${RESET}"
    echo -e "${CYAN}  ║${RESET}   ${BOLD}SmartopolAI — Setup v${VERSION}${RESET}           ${CYAN}║${RESET}"
    echo -e "${CYAN}  ║${RESET}   Autonomous AI gateway in Rust          ${CYAN}║${RESET}"
    echo -e "${CYAN}  ║${RESET}   Self-hosted · Privacy-first            ${CYAN}║${RESET}"
    echo -e "${CYAN}  ╚═══════════════════════════════════════════╝${RESET}"
    echo
}

# ─── 2. OS Detection ──────────────────────────────────────────────────────────
detect_os() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Linux)  ;;
        Darwin) ;;
        CYGWIN*|MINGW*|MSYS*|Windows_NT)
            die "Windows is not supported natively.
  Please use WSL2: https://learn.microsoft.com/en-us/windows/wsl/install
  Then run this script inside WSL2."
            ;;
        *)
            die "Unsupported operating system: $OS"
            ;;
    esac

    info "OS: ${BOLD}${OS}${RESET} / arch: ${BOLD}${ARCH}${RESET}"
}

# ─── 3. Dependency Check ──────────────────────────────────────────────────────
check_dependencies() {
    info "Checking dependencies..."

    if ! command -v git &>/dev/null; then
        die "git is required but not installed.
  macOS:  xcode-select --install   OR   brew install git
  Ubuntu: sudo apt install git
  Fedora: sudo dnf install git"
    fi

    if ! command -v curl &>/dev/null; then
        die "curl is required but not installed.
  macOS:  brew install curl
  Ubuntu: sudo apt install curl
  Fedora: sudo dnf install curl"
    fi

    # Rust — install via rustup if missing
    if ! command -v rustc &>/dev/null; then
        warn "Rust not found. Installing via rustup..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
        # shellcheck source=/dev/null
        source "$HOME/.cargo/env"
    fi

    if ! command -v cargo &>/dev/null; then
        # Try sourcing cargo env before giving up
        # shellcheck source=/dev/null
        [[ -f "$HOME/.cargo/env" ]] && source "$HOME/.cargo/env"
        command -v cargo &>/dev/null || die "cargo not found after rustup install. Restart your shell and try again."
    fi

    local rust_ver
    rust_ver=$(rustc --version | awk '{print $2}')
    if ! version_gte "$rust_ver" "1.${MIN_RUST_MINOR}"; then
        warn "Rust ${rust_ver} found, but 1.${MIN_RUST_MINOR}+ is required. Updating..."
        rustup update stable
        # shellcheck source=/dev/null
        source "$HOME/.cargo/env"
    fi

    success "Dependencies OK (Rust $(rustc --version | awk '{print $2}'))"
}

# ─── 4. Build ─────────────────────────────────────────────────────────────────
build_binary() {
    local skynet_src="$SCRIPT_DIR/skynet"

    if [[ ! -d "$skynet_src" ]]; then
        die "skynet/ directory not found at $skynet_src
  Run setup.sh from the repository root."
    fi

    info "Building SmartopolAI (first build may take a few minutes)..."

    (
        cd "$skynet_src"
        CARGO_TERM_COLOR=always cargo build --release 2>&1 | tee "$BUILD_LOG"
    )

    local binary_src="$skynet_src/target/release/$BINARY_NAME"
    if [[ ! -f "$binary_src" ]]; then
        die "Build failed. See $BUILD_LOG for details."
    fi

    mkdir -p "$SKYNET_DIR"
    cp "$binary_src" "$SKYNET_DIR/$BINARY_NAME"
    chmod +x "$SKYNET_DIR/$BINARY_NAME"

    success "Binary installed → $SKYNET_DIR/$BINARY_NAME"
}

# ─── 5. Create ~/.skynet/ ─────────────────────────────────────────────────────
create_skynet_dir() {
    mkdir -p "$SKYNET_DIR/tools"

    if [[ ! -f "$SOUL_DEST" ]]; then
        if [[ -f "$SOUL_TEMPLATE" ]]; then
            cp "$SOUL_TEMPLATE" "$SOUL_DEST"
            success "SOUL.md installed → $SOUL_DEST"
        else
            warn "SOUL template not found at $SOUL_TEMPLATE — skipping"
        fi
    else
        info "SOUL.md already exists, leaving unchanged."
    fi

    success "~/.skynet/ directory ready"
}

# ─── 6. Interactive Wizard ────────────────────────────────────────────────────
wizard() {
    echo
    echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
    echo -e "${BOLD}  Configuration Wizard${RESET}"
    echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
    echo

    # ── Step 1: AI Provider ──────────────────────────────────────────────────
    wizard_provider

    # ── Step 2: Auth Token ───────────────────────────────────────────────────
    echo -e "${BOLD}Step 2 — Gateway Auth Token${RESET}"
    local auto_token
    auto_token=$(generate_token)
    echo -e "  Press Enter to use auto-generated token: ${CYAN}${auto_token:0:14}...${RESET}"
    echo
    prompt AUTH_TOKEN "Token" "$auto_token"
    success "Auth token set (${#AUTH_TOKEN} characters)"
    echo

    # ── Step 3: Port ─────────────────────────────────────────────────────────
    echo -e "${BOLD}Step 3 — Gateway Port${RESET}"
    prompt GATEWAY_PORT "Port" "18789"
    success "Port: ${BOLD}${GATEWAY_PORT}${RESET}"
    echo

    # ── Step 4: Discord (optional) ───────────────────────────────────────────
    echo -e "${BOLD}Step 4 — Discord Bot ${CYAN}(optional — press Enter to skip)${RESET}${BOLD}${RESET}"
    local discord_yn=""
    echo -ne "  Enable Discord bot? [y/N]: "
    read -r discord_yn
    discord_yn="${discord_yn:-N}"
    echo

    DISCORD_TOML=""
    if [[ "$discord_yn" =~ ^[Yy] ]]; then
        echo -e "  ${BOLD}How to get a Discord bot token:${RESET}"
        echo "    1. Go to https://discord.com/developers/applications"
        echo "    2. Click 'New Application' — name it SmartopolAI (or anything)"
        echo "    3. Open the 'Bot' tab → 'Add Bot' → 'Reset Token'"
        echo "    4. Copy the token shown (you will not see it again)"
        echo
        local discord_token=""
        while [[ -z "$discord_token" ]]; do
            prompt_secret discord_token "Discord bot token"
            [[ -n "$discord_token" ]] || warn "Token cannot be empty."
        done

        local require_mention_val="false"
        local dm_allowed_val="true"
        local mention_yn=""
        echo -ne "  Require @mention in servers? [y/N]: "
        read -r mention_yn
        [[ "$mention_yn" =~ ^[Yy] ]] && require_mention_val="true"

        DISCORD_TOML="[channels.discord]
bot_token      = \"${discord_token}\"
require_mention = ${require_mention_val}
dm_allowed      = ${dm_allowed_val}"

        success "Discord configured."
        echo
        echo -e "  ${BOLD}Bot invite URL${RESET} (replace CLIENT_ID with your Application ID):"
        echo -e "  ${CYAN}https://discord.com/api/oauth2/authorize?client_id=CLIENT_ID&permissions=274878000128&scope=bot${RESET}"
        echo -e "  ${YELLOW}Find CLIENT_ID in the 'OAuth2' tab of your Discord application.${RESET}"
    fi
    echo
}

# ─── 7. Write Config ──────────────────────────────────────────────────────────
write_config() {
    echo -e "${BOLD}Writing configuration...${RESET}"

    local discord_block=""
    if [[ -n "$DISCORD_TOML" ]]; then
        discord_block="
${DISCORD_TOML}"
    fi

    cat > "$CONFIG_DEST" <<CONFIG
# SmartopolAI v${VERSION} — generated by setup.sh on $(date -u +"%Y-%m-%d %H:%M UTC")
# Edit this file to change any setting. Restart the gateway to apply.

[gateway]
port      = ${GATEWAY_PORT}
bind      = "127.0.0.1"
soul_path = "${SOUL_DEST}"

[gateway.auth]
mode  = "token"
token = "${AUTH_TOKEN}"

[agent]
model    = "${AGENT_MODEL}"
provider = "${PROVIDER_NAME}"

${PROVIDER_TOML}
${discord_block}
CONFIG

    success "Config written → $CONFIG_DEST"
    echo
}

# ─── 8. Health Check ──────────────────────────────────────────────────────────
health_check() {
    info "Running health check on port ${GATEWAY_PORT}..."

    "$SKYNET_DIR/$BINARY_NAME" >> "$LOG_FILE" 2>&1 &
    local pid=$!

    local ok=false
    local i
    for i in $(seq 1 12); do
        sleep 1
        if curl -sf "http://127.0.0.1:${GATEWAY_PORT}/health" &>/dev/null; then
            ok=true
            break
        fi
    done

    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true

    if $ok; then
        success "Health check passed — SmartopolAI v${VERSION} is operational"
    else
        warn "Health check did not respond on port ${GATEWAY_PORT}."
        warn "Check $LOG_FILE for details. You can start manually:"
        warn "  $SKYNET_DIR/$BINARY_NAME"
    fi
}

# ─── 9. First-Run Marker ──────────────────────────────────────────────────────
mark_first_run() {
    touch "$SKYNET_DIR/.first-run"
}

# ─── 10. Summary ──────────────────────────────────────────────────────────────
print_summary() {
    echo
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
    echo -e "${BOLD}  Setup complete. Starting SmartopolAI...${RESET}"
    echo
    echo -e "  Your agent will introduce itself and walk you through:"
    echo -e "  · Auto-start on boot (systemd / launchd)"
    echo -e "  · Community plugin installation"
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
    echo
    echo -e "  ${BOLD}Useful commands:${RESET}"
    echo -e "  Start:   ${CYAN}$SKYNET_DIR/$BINARY_NAME${RESET}"
    echo -e "  Config:  ${CYAN}$CONFIG_DEST${RESET}"
    echo -e "  Logs:    ${CYAN}$LOG_FILE${RESET}"
    echo -e "  Health:  ${CYAN}curl http://127.0.0.1:${GATEWAY_PORT}/health${RESET}"
    echo
}

# ─── 11. Launch Agent ─────────────────────────────────────────────────────────
launch_agent() {
    info "Starting SmartopolAI in the background..."
    "$SKYNET_DIR/$BINARY_NAME" >> "$LOG_FILE" 2>&1 &
    disown $!
    success "SmartopolAI running (logs: $LOG_FILE)"
    echo
    echo -e "  Connect via WebSocket:"
    echo -e "  ${CYAN}ws://127.0.0.1:${GATEWAY_PORT}/ws${RESET}"
    echo
}

# ─── 12. Terminal REPL ────────────────────────────────────────────────────────
repl_chat() {
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
    echo -e "${BOLD}  SmartopolAI Terminal${RESET}"
    echo -e "  Type your message and press Enter."
    echo -e "  ${CYAN}/setup-model${RESET} — switch AI provider  ·  ${CYAN}/exit${RESET} — quit"
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
    echo

    # Wait until gateway is ready (already health-checked, just re-confirm)
    local i
    for i in $(seq 1 8); do
        curl -sf "http://127.0.0.1:${GATEWAY_PORT}/health" &>/dev/null && break
        sleep 1
    done

    while true; do
        echo -ne "${BOLD}You:${RESET} "
        local user_input
        read -r user_input

        [[ -z "$user_input" ]] && continue

        # ── Slash commands ───────────────────────────────────────────────────
        if [[ "$user_input" == "/exit" || "$user_input" == "exit" || "$user_input" == "quit" ]]; then
            echo
            info "Chat closed. Gateway is still running in the background."
            echo -e "  Logs: ${CYAN}${LOG_FILE}${RESET}"
            break
        fi

        if [[ "$user_input" == "/setup-model" ]]; then
            echo
            warn "Reconfiguring AI provider..."
            wizard_provider
            write_config
            pkill -f "$BINARY_NAME" 2>/dev/null || true
            sleep 1
            "$SKYNET_DIR/$BINARY_NAME" >> "$LOG_FILE" 2>&1 &
            disown $!
            sleep 2
            success "Gateway restarted with provider: ${BOLD}${PROVIDER_NAME}${RESET}"
            echo
            continue
        fi

        # ── Send message via POST /chat ──────────────────────────────────────
        local json_body
        if ! json_body=$(python3 -c \
            "import json,sys; print(json.dumps({'message': sys.argv[1]}))" \
            "$user_input" 2>/dev/null); then
            warn "python3 not found — cannot encode message safely."
            continue
        fi

        local raw http_code body reply err
        raw=$(curl -s \
            -X POST \
            -H "Content-Type: application/json" \
            -H "Authorization: Bearer ${AUTH_TOKEN}" \
            -w "\n%{http_code}" \
            -d "$json_body" \
            "http://127.0.0.1:${GATEWAY_PORT}/chat" 2>/dev/null)

        http_code=$(printf '%s' "$raw" | tail -n1)
        body=$(printf '%s' "$raw" | head -n -1)

        case "$http_code" in
            200)
                reply=$(python3 -c \
                    "import json,sys; d=json.load(sys.stdin); print(d.get('reply',''))" \
                    <<< "$body" 2>/dev/null) || reply="$body"
                echo
                echo -e "${CYAN}SmartopolAI:${RESET} ${reply}"
                echo
                ;;
            401)
                warn "Authentication failed. Check your token in ${CONFIG_DEST}"
                warn "Or type /setup-model to reconfigure."
                ;;
            500)
                err=$(python3 -c \
                    "import json,sys; d=json.load(sys.stdin); print(d.get('error','AI error'))" \
                    <<< "$body" 2>/dev/null) || err="Internal error"
                echo
                warn "AI error: ${err}"
                warn "Check your API key in ${CONFIG_DEST} or type /setup-model."
                echo
                ;;
            *)
                warn "Gateway unreachable (HTTP ${http_code:-no response})."
                warn "Check logs: ${LOG_FILE}"
                warn "Or type /setup-model to reconfigure your AI provider."
                ;;
        esac
    done
}

# ─── Main ─────────────────────────────────────────────────────────────────────
main() {
    print_banner
    detect_os
    check_dependencies
    build_binary
    create_skynet_dir
    wizard
    write_config
    health_check
    mark_first_run
    print_summary
    launch_agent
    repl_chat
}

main "$@"
