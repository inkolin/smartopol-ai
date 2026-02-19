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
        anthropic-subscription)
            # Claude Max/Pro subscription tokens use Bearer + OAuth beta header
            status=$(curl -s -o /dev/null -w "%{http_code}" \
                --max-time 10 \
                -H "Authorization: Bearer $key" \
                -H "anthropic-version: 2023-06-01" \
                -H "anthropic-beta: oauth-2025-04-20" \
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
        openai-compat)
            # Generic OpenAI-compatible — just check that the endpoint responds
            if [[ -z "$base_url" ]]; then return 1; fi
            status=$(curl -s -o /dev/null -w "%{http_code}" \
                --max-time 10 \
                -H "Authorization: Bearer $key" \
                -H "Content-Type: application/json" \
                -d '{"model":"test","messages":[{"role":"user","content":"hi"}],"max_tokens":1}' \
                "${base_url}")
            # Accept 200, 400 (bad model), 401 means bad key
            [[ "$status" == "200" || "$status" == "400" || "$status" == "404" ]]
            ;;
        ollama)
            status=$(curl -s -o /dev/null -w "%{http_code}" \
                --max-time 5 \
                "${base_url}/api/tags")
            [[ "$status" == "200" ]]
            ;;
    esac
}

# ─── Provider registry (mirrors skynet-agent/src/registry.rs) ─────────────────
# Each line: ID|DISPLAY_NAME|BASE_URL|CHAT_PATH|DEFAULT_MODEL|FREE
REGISTRY=(
    "groq|Groq|https://api.groq.com/openai|/v1/chat/completions|llama-3.3-70b-versatile|free"
    "deepseek|DeepSeek|https://api.deepseek.com|/v1/chat/completions|deepseek-chat|"
    "openrouter|OpenRouter (200+ models)|https://openrouter.ai/api|/v1/chat/completions|openai/gpt-4o|free"
    "xai|xAI (Grok)|https://api.x.ai|/v1/chat/completions|grok-2-latest|"
    "mistral|Mistral AI|https://api.mistral.ai|/v1/chat/completions|mistral-large-latest|"
    "perplexity|Perplexity|https://api.perplexity.ai|/chat/completions|sonar-pro|"
    "together|Together AI|https://api.together.xyz|/v1/chat/completions|meta-llama/Meta-Llama-3.1-70B-Instruct-Turbo|"
    "fireworks|Fireworks AI|https://api.fireworks.ai/inference|/v1/chat/completions|accounts/fireworks/models/llama-v3p3-70b-instruct|"
    "cerebras|Cerebras|https://api.cerebras.ai|/v1/chat/completions|llama3.3-70b|free"
    "sambanova|SambaNova|https://api.sambanova.ai|/v1/chat/completions|Meta-Llama-3.1-405B-Instruct|free"
    "hyperbolic|Hyperbolic|https://api.hyperbolic.xyz|/v1/chat/completions|meta-llama/Llama-3.3-70B-Instruct|"
    "novita|Novita AI|https://api.novita.ai/v3/openai|/chat/completions|meta-llama/llama-3.1-70b-instruct|"
    "lepton|Lepton AI|https://llm.lepton.ai/api|/v1/chat/completions|llama3-3-70b|free"
    "corethink|CoreThink|https://api.corethink.ai|/v1/chat/completions|deepseek-r1|"
    "featherless|Featherless AI|https://api.featherless.ai|/v1/chat/completions|meta-llama/Meta-Llama-3.1-70B-Instruct|"
    "requesty|Requesty|https://router.requesty.ai|/v1/chat/completions|openai/gpt-4o|"
    "glama|Glama|https://glama.ai/api|/v1/chat/completions|openai/gpt-4o|free"
    "chutes|Chutes AI|https://llm.chutes.ai|/v1/chat/completions|deepseek-ai/DeepSeek-R1|free"
    "cohere|Cohere|https://api.cohere.com/compatibility|/v1/chat/completions|command-r-plus-08-2024|free"
    "moonshot|Moonshot AI (Kimi)|https://api.moonshot.cn|/v1/chat/completions|moonshot-v1-8k|"
    "glm|GLM (Zhipu AI)|https://open.bigmodel.cn/api/paas|/v4/chat/completions|glm-4-flash|free"
    "doubao|Doubao (ByteDance)|https://ark.cn-beijing.volces.com/api|/v3/chat/completions|doubao-pro-4k|"
    "qwen|Qwen (Alibaba)|https://dashscope.aliyuncs.com/compatible-mode|/v1/chat/completions|qwen-turbo|"
    "zai|Z.AI|https://api.z.ai|/v1/chat/completions|z1-preview|"
    "yi|01.AI (Yi)|https://api.01.ai|/v1/chat/completions|yi-large|"
    "minimax|MiniMax|https://api.minimax.chat|/v1/text/chatcompletion_v2|MiniMax-Text-01|"
    "hunyuan|Hunyuan (Tencent)|https://api.hunyuan.cloud.tencent.com|/v1/chat/completions|hunyuan-turbo|"
    "stepfun|StepFun|https://api.stepfun.com|/v1/chat/completions|step-1-8k|"
    "gemini|Google AI (Gemini)|https://generativelanguage.googleapis.com/v1beta/openai|/chat/completions|gemini-2.0-flash|free"
    "lmstudio|LM Studio (local)|http://localhost:1234|/v1/chat/completions|local-model|free"
    "llamacpp|llama.cpp server (local)|http://localhost:8080|/v1/chat/completions|local-model|free"
    "localai|LocalAI (local)|http://localhost:8080|/v1/chat/completions|gpt-4|free"
    "litellm|LiteLLM proxy|http://localhost:4000|/v1/chat/completions|gpt-3.5-turbo|free"
)

# Lookup registry entry by ID, returns pipe-delimited string or empty
registry_lookup() {
    local id="$1"
    for entry in "${REGISTRY[@]}"; do
        if [[ "${entry%%|*}" == "$id" ]]; then
            echo "$entry"
            return 0
        fi
    done
    return 1
}

# ─── OAuth Device Flow: GitHub Copilot ────────────────────────────────────────
# Runs the GitHub device code flow, saves the long-lived GitHub access token
# to ~/.skynet/copilot_token. The Rust runtime reads this file and exchanges
# it for short-lived Copilot API keys automatically.
COPILOT_CLIENT_ID="Iv1.b507a08c87ecfe98"
COPILOT_TOKEN_FILE="$SKYNET_DIR/copilot_token"

oauth_github_copilot() {
    info "Starting GitHub Copilot device authorization..."
    echo

    # Step 1: Request device code
    local resp
    resp=$(curl -s -X POST \
        -H "Accept: application/json" \
        -d "client_id=${COPILOT_CLIENT_ID}&scope=read:user" \
        "https://github.com/login/device/code")

    local device_code user_code verification_uri interval
    device_code=$(printf '%s' "$resp" | python3 -c "import json,sys; print(json.load(sys.stdin)['device_code'])" 2>/dev/null)
    user_code=$(printf '%s' "$resp" | python3 -c "import json,sys; print(json.load(sys.stdin)['user_code'])" 2>/dev/null)
    verification_uri=$(printf '%s' "$resp" | python3 -c "import json,sys; print(json.load(sys.stdin)['verification_uri'])" 2>/dev/null)
    interval=$(printf '%s' "$resp" | python3 -c "import json,sys; print(json.load(sys.stdin).get('interval',5))" 2>/dev/null)

    if [[ -z "$device_code" || -z "$user_code" ]]; then
        warn "Failed to get device code from GitHub."
        return 1
    fi

    echo -e "  ${BOLD}Open this URL in your browser:${RESET}"
    echo -e "  ${CYAN}${verification_uri}${RESET}"
    echo
    echo -e "  ${BOLD}Enter this code:${RESET}  ${GREEN}${user_code}${RESET}"
    echo
    echo -e "  Waiting for authorization..."

    # Step 2: Poll for token
    local max_attempts=60
    local attempt=0
    local access_token=""

    while [[ $attempt -lt $max_attempts ]]; do
        sleep "${interval}"
        ((attempt++))

        local token_resp
        token_resp=$(curl -s -X POST \
            -H "Accept: application/json" \
            -d "client_id=${COPILOT_CLIENT_ID}&device_code=${device_code}&grant_type=urn:ietf:params:oauth:grant-type:device_code" \
            "https://github.com/login/oauth/access_token")

        local error
        error=$(printf '%s' "$token_resp" | python3 -c "import json,sys; print(json.load(sys.stdin).get('error',''))" 2>/dev/null)

        case "$error" in
            authorization_pending) continue ;;
            slow_down) interval=$((interval + 5)); continue ;;
            "")
                access_token=$(printf '%s' "$token_resp" | python3 -c "import json,sys; print(json.load(sys.stdin).get('access_token',''))" 2>/dev/null)
                if [[ -n "$access_token" ]]; then
                    break
                fi
                ;;
            *)
                warn "GitHub OAuth error: $error"
                return 1
                ;;
        esac
    done

    if [[ -z "$access_token" ]]; then
        warn "Authorization timed out."
        return 1
    fi

    # Step 3: Save token to disk
    mkdir -p "$SKYNET_DIR"
    printf '%s' "$access_token" > "$COPILOT_TOKEN_FILE"
    chmod 600 "$COPILOT_TOKEN_FILE"

    success "GitHub Copilot authorized — token saved to ${COPILOT_TOKEN_FILE}"
    return 0
}

# ─── OAuth Device Flow: Qwen (Alibaba) ───────────────────────────────────────
# Runs the Qwen device code flow with PKCE, saves credentials JSON to
# ~/.skynet/qwen_credentials.json. The Rust runtime reads this file and
# refreshes the token automatically when expired.
QWEN_CLIENT_ID="f0304373b74a44d2b584a3fb70ca9e56"
QWEN_CREDENTIALS_FILE="$SKYNET_DIR/qwen_credentials.json"

oauth_qwen() {
    info "Starting Qwen device authorization (2000 free requests/day)..."
    echo

    # Step 1: Generate PKCE code_verifier + code_challenge (S256)
    local code_verifier code_challenge
    code_verifier=$(head -c 32 /dev/urandom | base64 | tr -d '=/+' | head -c 43)
    code_challenge=$(printf '%s' "$code_verifier" | openssl dgst -sha256 -binary | base64 | tr '+/' '-_' | tr -d '=')

    # Step 2: Request device code
    local request_id
    request_id=$(python3 -c "import uuid; print(uuid.uuid4())" 2>/dev/null || echo "setup-$$")

    local resp
    resp=$(curl -s -X POST \
        -H "Content-Type: application/x-www-form-urlencoded" \
        -H "Accept: application/json" \
        -H "x-request-id: ${request_id}" \
        -d "client_id=${QWEN_CLIENT_ID}&scope=openid+profile+email+model.completion&code_challenge=${code_challenge}&code_challenge_method=S256" \
        "https://chat.qwen.ai/api/v1/oauth2/device/code")

    local device_code user_code verification_uri verification_uri_complete interval
    device_code=$(printf '%s' "$resp" | python3 -c "import json,sys; print(json.load(sys.stdin)['device_code'])" 2>/dev/null)
    user_code=$(printf '%s' "$resp" | python3 -c "import json,sys; print(json.load(sys.stdin)['user_code'])" 2>/dev/null)
    verification_uri=$(printf '%s' "$resp" | python3 -c "import json,sys; print(json.load(sys.stdin).get('verification_uri',''))" 2>/dev/null)
    verification_uri_complete=$(printf '%s' "$resp" | python3 -c "import json,sys; print(json.load(sys.stdin).get('verification_uri_complete',''))" 2>/dev/null)
    interval=$(printf '%s' "$resp" | python3 -c "import json,sys; print(json.load(sys.stdin).get('interval',5))" 2>/dev/null)

    if [[ -z "$device_code" || -z "$user_code" ]]; then
        warn "Failed to get device code from Qwen."
        warn "Response: $resp"
        return 1
    fi

    # Use verification_uri_complete (has user_code embedded) if available,
    # otherwise append user_code as query parameter.
    local open_url=""
    if [[ -n "$verification_uri_complete" ]]; then
        open_url="$verification_uri_complete"
    elif [[ -n "$verification_uri" ]]; then
        open_url="${verification_uri}?user_code=${user_code}"
    else
        open_url="https://chat.qwen.ai/authorize?user_code=${user_code}"
    fi

    echo -e "  ${BOLD}Open this URL in your browser:${RESET}"
    echo -e "  ${CYAN}${open_url}${RESET}"
    echo
    echo -e "  ${BOLD}Code:${RESET}  ${GREEN}${user_code}${RESET}"
    echo
    echo -e "  Waiting for authorization..."

    # Step 3: Poll for token
    local max_attempts=60
    local attempt=0
    local token_json=""

    while [[ $attempt -lt $max_attempts ]]; do
        sleep "${interval}"
        ((attempt++))

        local token_resp
        token_resp=$(curl -s -X POST \
            -H "Content-Type: application/x-www-form-urlencoded" \
            -d "client_id=${QWEN_CLIENT_ID}&device_code=${device_code}&code_verifier=${code_verifier}&grant_type=urn:ietf:params:oauth:grant-type:device_code" \
            "https://chat.qwen.ai/api/v1/oauth2/token")

        local error
        error=$(printf '%s' "$token_resp" | python3 -c "import json,sys; print(json.load(sys.stdin).get('error',''))" 2>/dev/null)

        case "$error" in
            authorization_pending) continue ;;
            slow_down) interval=$((interval + 5)); continue ;;
            "")
                local access_token
                access_token=$(printf '%s' "$token_resp" | python3 -c "import json,sys; print(json.load(sys.stdin).get('access_token',''))" 2>/dev/null)
                if [[ -n "$access_token" ]]; then
                    token_json="$token_resp"
                    break
                fi
                ;;
            *)
                warn "Qwen OAuth error: $error"
                return 1
                ;;
        esac
    done

    if [[ -z "$token_json" ]]; then
        warn "Authorization timed out."
        return 1
    fi

    # Step 4: Build credentials JSON and save to disk
    mkdir -p "$SKYNET_DIR"
    python3 -c "
import json, sys, time
resp = json.loads(sys.argv[1])
creds = {
    'access_token': resp['access_token'],
    'refresh_token': resp.get('refresh_token', ''),
    'token_type': resp.get('token_type', 'Bearer'),
    'expiry_date': int(time.time() * 1000) + resp.get('expires_in', 3600) * 1000,
}
print(json.dumps(creds, indent=2))
" "$token_json" > "$QWEN_CREDENTIALS_FILE"
    chmod 600 "$QWEN_CREDENTIALS_FILE"

    success "Qwen authorized — credentials saved to ${QWEN_CREDENTIALS_FILE}"
    return 0
}

# Step 1 of wizard extracted so /setup-model can call it standalone.
# Loops until a valid provider+key is confirmed.
wizard_provider() {
    while true; do
        echo -e "${BOLD}Step 1 — How will you access the AI?${RESET}"
        echo
        echo -e "    1) ${BOLD}API key${RESET} — I have a pay-per-use key ${CYAN}(most common)${RESET}"
        echo -e "    2) ${BOLD}Subscription${RESET} — Claude Max, GitHub Copilot, Qwen free"
        echo -e "    3) ${BOLD}Local${RESET} — Ollama / LM Studio (free, runs on your machine)"
        echo -e "    4) ${BOLD}Enterprise${RESET} — AWS Bedrock / Google Vertex AI"
        echo
        local access_choice=""
        prompt access_choice "Choice" "1"
        echo

        local api_key="" done=false

        case "$access_choice" in
        # ═══════════════════════════════════════════════════════════════════════
        # API KEY flow
        # ═══════════════════════════════════════════════════════════════════════
        1)
            while true; do
                echo -e "${BOLD}  Choose your AI provider:${RESET}"
                echo
                echo -e "    ${CYAN}── Popular ──${RESET}"
                echo -e "     1) Anthropic Claude  ${CYAN}(recommended)${RESET}"
                echo    "     2) OpenAI GPT"
                echo -e "     3) Groq              ${GREEN}(free tier)${RESET}"
                echo    "     4) DeepSeek          (very cheap)"
                echo -e "     5) OpenRouter        ${GREEN}(200+ models, one key)${RESET}"
                echo    "     6) xAI (Grok)"
                echo    "     7) Mistral AI"
                echo    "     8) Perplexity"
                echo
                echo -e "    ${CYAN}── More providers ──${RESET}"
                echo -e "     9) Google Gemini     ${GREEN}(free tier)${RESET}"
                echo    "    10) Together AI       16) Moonshot (Kimi)"
                echo -e "    11) Fireworks AI      17) GLM / Zhipu AI  ${GREEN}(free)${RESET}"
                echo -e "    12) Cerebras          ${GREEN}(free)${RESET}  18) Doubao (ByteDance)"
                echo -e "    13) SambaNova         ${GREEN}(free)${RESET}  19) Qwen (Alibaba)"
                echo    "    14) Cohere            20) Z.AI"
                echo    "    15) Hyperbolic        21) 01.AI (Yi)"
                echo
                echo -e "    ${CYAN}── Other ──${RESET}"
                echo    "    22) MiniMax / Hunyuan / StepFun / Novita / Lepton / etc."
                echo    "    23) Custom OpenAI-compatible endpoint"
                echo -e "     0) ${YELLOW}Back${RESET}"
                echo
                local pc=""
                prompt pc "Provider" "1"
                echo

                # map numeric to registry ID
                local reg_id="" reg_name="" reg_base="" reg_path="" reg_model=""
                case "$pc" in
                    1)  PROVIDER_NAME="anthropic"; AGENT_MODEL="claude-sonnet-4-6" ;;
                    2)  PROVIDER_NAME="openai"; AGENT_MODEL="gpt-4o" ;;
                    3)  reg_id="groq" ;;
                    4)  reg_id="deepseek" ;;
                    5)  reg_id="openrouter" ;;
                    6)  reg_id="xai" ;;
                    7)  reg_id="mistral" ;;
                    8)  reg_id="perplexity" ;;
                    9)  reg_id="gemini" ;;
                    10) reg_id="together" ;;
                    11) reg_id="fireworks" ;;
                    12) reg_id="cerebras" ;;
                    13) reg_id="sambanova" ;;
                    14) reg_id="cohere" ;;
                    15) reg_id="hyperbolic" ;;
                    16) reg_id="moonshot" ;;
                    17) reg_id="glm" ;;
                    18) reg_id="doubao" ;;
                    19) reg_id="qwen" ;;
                    20) reg_id="zai" ;;
                    22)
                        echo -e "  ${BOLD}Enter provider ID:${RESET} (minimax, hunyuan, stepfun, novita,"
                        echo    "  lepton, corethink, featherless, requesty, glama, chutes)"
                        local sub_id=""
                        prompt sub_id "ID" ""
                        reg_id="$sub_id"
                        ;;
                    23)
                        # Custom endpoint
                        PROVIDER_NAME="custom"
                        local custom_name="" custom_url="" custom_model=""
                        prompt custom_name "Provider name (for logs)" "custom"
                        prompt custom_url "Base URL (e.g. https://my-server.com)" ""
                        prompt custom_model "Model name" "default"
                        prompt_secret api_key "API key"
                        if [[ -n "$custom_url" && -n "$api_key" ]]; then
                            PROVIDER_NAME="$custom_name"
                            AGENT_MODEL="$custom_model"
                            PROVIDER_TOML="[[providers.openai_compat]]
id        = \"${custom_name}\"
api_key   = \"${api_key}\"
base_url  = \"${custom_url}\""
                            done=true
                        else
                            warn "URL and API key are required."
                        fi
                        ;;
                    0) break ;;
                    *) warn "Invalid choice."; continue ;;
                esac

                # Custom handled above — skip rest
                $done && break

                # ── Anthropic (native) ────────────────────────────────────────
                if [[ "$PROVIDER_NAME" == "anthropic" ]]; then
                    while true; do
                        prompt_secret api_key "Anthropic API key (sk-ant-...)"
                        if [[ ! "$api_key" =~ ^sk-ant- ]]; then
                            warn "Anthropic keys start with 'sk-ant-'. Try again, or type 'back'."
                            local cmd; read -r cmd
                            [[ "$cmd" == "back" ]] && break
                            continue
                        fi
                        if validate_api_key "anthropic" "$api_key"; then
                            success "Anthropic API key accepted"
                            PROVIDER_TOML="[providers.anthropic]
api_key = \"${api_key}\""
                            done=true; break
                        else
                            echo
                            warn "Anthropic rejected this key."
                            warn "Get yours at: https://console.anthropic.com"
                            echo -ne "  Press Enter to retry, or type ${CYAN}back${RESET}: "
                            local cmd; read -r cmd
                            [[ "$cmd" == "back" ]] && break
                        fi
                    done
                    $done && break
                    continue
                fi

                # ── OpenAI (native) ───────────────────────────────────────────
                if [[ "$PROVIDER_NAME" == "openai" ]]; then
                    while true; do
                        prompt_secret api_key "OpenAI API key (sk-...)"
                        if [[ ! "$api_key" =~ ^sk- ]]; then
                            warn "OpenAI keys start with 'sk-'. Try again, or type 'back'."
                            local cmd; read -r cmd
                            [[ "$cmd" == "back" ]] && break
                            continue
                        fi
                        if validate_api_key "openai" "$api_key"; then
                            success "OpenAI API key accepted"
                            PROVIDER_TOML="[providers.openai]
api_key = \"${api_key}\""
                            done=true; break
                        else
                            echo
                            warn "OpenAI rejected this key (wrong key or billing issue)."
                            warn "Get yours at: https://platform.openai.com/api-keys"
                            echo -ne "  Press Enter to retry, or type ${CYAN}back${RESET}: "
                            local cmd; read -r cmd
                            [[ "$cmd" == "back" ]] && break
                        fi
                    done
                    $done && break
                    continue
                fi

                # ── OpenAI-compatible from registry ───────────────────────────
                if [[ -n "$reg_id" ]]; then
                    local entry
                    if ! entry=$(registry_lookup "$reg_id"); then
                        warn "Unknown provider '${reg_id}'. Use option 22 for custom endpoint."
                        continue
                    fi
                    IFS='|' read -r _ reg_name reg_base reg_path reg_model _ <<< "$entry"
                    PROVIDER_NAME="$reg_id"
                    AGENT_MODEL="$reg_model"

                    prompt_secret api_key "${reg_name} API key"
                    if [[ -z "$api_key" ]]; then
                        warn "API key cannot be empty."
                        continue
                    fi

                    local endpoint="${reg_base}${reg_path}"
                    if validate_api_key "openai-compat" "$api_key" "$endpoint"; then
                        success "${reg_name} connection verified"
                    else
                        warn "Could not verify ${reg_name} — key might be wrong or server unreachable."
                        warn "Continuing anyway — fix in ${CONFIG_DEST} if needed."
                    fi

                    PROVIDER_TOML="[[providers.openai_compat]]
id      = \"${reg_id}\"
api_key = \"${api_key}\""
                    done=true; break
                fi
            done
            ;;

        # ═══════════════════════════════════════════════════════════════════════
        # SUBSCRIPTION flow
        # ═══════════════════════════════════════════════════════════════════════
        2)
            echo -e "${BOLD}  Which subscription do you have?${RESET}"
            echo
            echo -e "    1) ${BOLD}Anthropic Claude Max / Pro${RESET} ${CYAN}(recommended)${RESET}"
            echo    "       Uses a subscription token (sk-ant-oat01-...)"
            echo    "       Get it: Settings → API → Create Setup Token"
            echo
            echo -e "    2) ${BOLD}GitHub Copilot${RESET}"
            echo    "       Free for open-source, included with GitHub Pro"
            echo    "       Browser-based login (no key needed)"
            echo
            echo -e "    3) ${BOLD}Qwen (Alibaba)${RESET} ${GREEN}(2000 free req/day)${RESET}"
            echo    "       Browser-based login (no key needed)"
            echo
            echo -e "    0) ${YELLOW}Back${RESET}"
            echo
            local sub_choice=""
            prompt sub_choice "Choice" "1"
            echo

            case "$sub_choice" in
                1)
                    PROVIDER_NAME="anthropic"
                    AGENT_MODEL="claude-sonnet-4-6"
                    while true; do
                        prompt_secret api_key "Subscription token (sk-ant-oat01-...)"
                        if [[ ! "$api_key" =~ ^sk-ant-oat01- ]]; then
                            warn "Subscription tokens start with 'sk-ant-oat01-'."
                            warn "Type 'back' to go back, or try again."
                            local cmd; read -r cmd
                            [[ "$cmd" == "back" ]] && break
                            continue
                        fi
                        if validate_api_key "anthropic-subscription" "$api_key"; then
                            success "Claude Max subscription verified"
                            PROVIDER_TOML="[providers.anthropic]
api_key = \"${api_key}\""
                            done=true; break
                        else
                            echo
                            warn "Subscription token rejected. Make sure you:"
                            warn "  1. Have an active Claude Max or Pro subscription"
                            warn "  2. Created a Setup Token at console.anthropic.com"
                            echo -ne "  Press Enter to retry, or type ${CYAN}back${RESET}: "
                            local cmd; read -r cmd
                            [[ "$cmd" == "back" ]] && break
                        fi
                    done
                    ;;
                2)
                    PROVIDER_NAME="copilot"
                    AGENT_MODEL="gpt-4o"
                    if oauth_github_copilot; then
                        PROVIDER_TOML="[providers.copilot]
token_path = \"${COPILOT_TOKEN_FILE}\""
                        done=true
                    fi
                    ;;
                3)
                    PROVIDER_NAME="qwen-oauth"
                    AGENT_MODEL="qwen-turbo"
                    if oauth_qwen; then
                        PROVIDER_TOML="[providers.qwen_oauth]
credentials_path = \"${QWEN_CREDENTIALS_FILE}\""
                        done=true
                    fi
                    ;;
                0) continue ;;
                *) warn "Invalid choice."; continue ;;
            esac
            ;;

        # ═══════════════════════════════════════════════════════════════════════
        # LOCAL flow
        # ═══════════════════════════════════════════════════════════════════════
        3)
            echo -e "${BOLD}  Which local AI server?${RESET}"
            echo
            echo -e "    1) Ollama            ${CYAN}(recommended)${RESET} — https://ollama.com"
            echo    "    2) LM Studio         — https://lmstudio.ai"
            echo    "    3) llama.cpp server"
            echo    "    4) LocalAI"
            echo    "    5) LiteLLM proxy"
            echo -e "    0) ${YELLOW}Back${RESET}"
            echo
            local local_choice=""
            prompt local_choice "Choice" "1"
            echo

            case "$local_choice" in
                1)
                    PROVIDER_NAME="ollama"
                    AGENT_MODEL="llama3.2"
                    local ollama_url=""
                    prompt ollama_url "Ollama base URL" "http://localhost:11434"
                    if validate_api_key "ollama" "" "$ollama_url"; then
                        success "Ollama server reachable at ${ollama_url}"
                    else
                        warn "Ollama not reachable at ${ollama_url}."
                        warn "Start it with: ollama serve  →  https://ollama.com"
                        warn "Continuing — fix the URL in ${CONFIG_DEST} when ready."
                    fi
                    PROVIDER_TOML="[providers.ollama]
base_url = \"${ollama_url}\""
                    done=true
                    ;;
                2) reg_id="lmstudio" ;;
                3) reg_id="llamacpp" ;;
                4) reg_id="localai" ;;
                5) reg_id="litellm" ;;
                0) continue ;;
                *) warn "Invalid choice."; continue ;;
            esac

            # Handle LM Studio / llama.cpp / LocalAI / LiteLLM
            if [[ -n "${reg_id:-}" ]] && ! $done; then
                local entry
                entry=$(registry_lookup "$reg_id")
                IFS='|' read -r _ reg_name reg_base reg_path reg_model _ <<< "$entry"
                PROVIDER_NAME="$reg_id"
                AGENT_MODEL="$reg_model"

                local local_url=""
                prompt local_url "${reg_name} URL" "$reg_base"

                local endpoint="${local_url}${reg_path}"
                if validate_api_key "openai-compat" "no-key-needed" "$endpoint"; then
                    success "${reg_name} reachable at ${local_url}"
                else
                    warn "${reg_name} not reachable at ${local_url}."
                    warn "Make sure the server is running. Continuing anyway."
                fi

                PROVIDER_TOML="[[providers.openai_compat]]
id       = \"${reg_id}\"
api_key  = \"local\"
base_url = \"${local_url}\""
                done=true
            fi
            ;;

        # ═══════════════════════════════════════════════════════════════════════
        # ENTERPRISE flow (AWS Bedrock / Google Vertex AI)
        # ═══════════════════════════════════════════════════════════════════════
        4)
            echo -e "${BOLD}  Enterprise cloud provider:${RESET}"
            echo
            echo -e "    1) ${BOLD}AWS Bedrock${RESET} (Claude, Llama, Mistral on AWS)"
            echo    "       Uses AWS credentials (~/.aws/credentials or env vars)"
            echo
            echo -e "    2) ${BOLD}Google Vertex AI${RESET} (Gemini models on GCP)"
            echo    "       Uses a service account JSON key file"
            echo
            echo -e "    0) ${YELLOW}Back${RESET}"
            echo
            local ent_choice=""
            prompt ent_choice "Choice" "1"
            echo

            case "$ent_choice" in
                1)
                    # AWS Bedrock
                    PROVIDER_NAME="bedrock"
                    AGENT_MODEL="anthropic.claude-3-5-sonnet-20241022-v2:0"
                    local aws_region=""
                    prompt aws_region "AWS region" "us-east-1"
                    local aws_profile=""
                    prompt aws_profile "AWS profile (from ~/.aws/credentials)" "default"

                    # Check if credentials are available
                    if [[ -n "${AWS_ACCESS_KEY_ID:-}" && -n "${AWS_SECRET_ACCESS_KEY:-}" ]]; then
                        success "AWS credentials found in environment"
                    elif [[ -f "$HOME/.aws/credentials" ]]; then
                        if grep -q "\[$aws_profile\]" "$HOME/.aws/credentials" 2>/dev/null; then
                            success "AWS profile '${aws_profile}' found in ~/.aws/credentials"
                        else
                            warn "Profile '${aws_profile}' not found in ~/.aws/credentials."
                            warn "Run: aws configure --profile ${aws_profile}"
                            warn "Continuing — fix before starting the gateway."
                        fi
                    else
                        warn "No AWS credentials found."
                        warn "Run: aws configure  OR  set AWS_ACCESS_KEY_ID + AWS_SECRET_ACCESS_KEY"
                        warn "Continuing — fix before starting the gateway."
                    fi

                    PROVIDER_TOML="[providers.bedrock]
region  = \"${aws_region}\""
                    if [[ "$aws_profile" != "default" ]]; then
                        PROVIDER_TOML="${PROVIDER_TOML}
profile = \"${aws_profile}\""
                    fi
                    done=true
                    ;;
                2)
                    # Google Vertex AI
                    PROVIDER_NAME="vertex"
                    AGENT_MODEL="gemini-2.0-flash"
                    local gcp_key_file="" gcp_project="" gcp_location=""
                    prompt gcp_key_file "Path to service account JSON key file" ""

                    if [[ -z "$gcp_key_file" ]]; then
                        warn "Service account key file is required."
                        warn "Create one at: console.cloud.google.com → IAM → Service Accounts → Keys"
                        continue
                    fi

                    if [[ ! -f "$gcp_key_file" ]]; then
                        warn "File not found: ${gcp_key_file}"
                        continue
                    fi

                    # Try to auto-detect project_id from the key file
                    local auto_project=""
                    auto_project=$(python3 -c "import json; print(json.load(open('$gcp_key_file')).get('project_id',''))" 2>/dev/null) || true
                    prompt gcp_project "GCP project ID" "${auto_project}"
                    prompt gcp_location "GCP region" "us-central1"

                    success "Vertex AI configured (project: ${gcp_project})"

                    PROVIDER_TOML="[providers.vertex]
key_file   = \"${gcp_key_file}\"
project_id = \"${gcp_project}\"
location   = \"${gcp_location}\""
                    done=true
                    ;;
                0) continue ;;
                *) warn "Invalid choice."; continue ;;
            esac
            ;;

        *)
            warn "Invalid choice."
            continue
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

# ─── 12. Send a message to the gateway and print the reply ────────────────────
# Returns 0 on success, 1 on error. Sets LAST_REPLY global.
LAST_REPLY=""
send_chat_message() {
    local message="$1"
    LAST_REPLY=""

    local json_body
    if ! json_body=$(python3 -c \
        "import json,sys; print(json.dumps({'message': sys.argv[1]}))" \
        "$message" 2>/dev/null); then
        warn "python3 not found — cannot encode message safely."
        return 1
    fi

    local raw http_code body
    raw=$(curl -s \
        -X POST \
        -H "Content-Type: application/json" \
        -H "Authorization: Bearer ${AUTH_TOKEN}" \
        -w "\n%{http_code}" \
        --max-time 60 \
        -d "$json_body" \
        "http://127.0.0.1:${GATEWAY_PORT}/chat" 2>/dev/null)

    http_code=$(printf '%s' "$raw" | tail -n1)
    body=$(printf '%s' "$raw" | sed '$d')

    case "$http_code" in
        200)
            LAST_REPLY=$(python3 -c \
                "import json,sys; d=json.load(sys.stdin); print(d.get('reply',''))" \
                <<< "$body" 2>/dev/null) || LAST_REPLY="$body"
            echo
            echo -e "${CYAN}SmartopolAI:${RESET} ${LAST_REPLY}"
            echo
            return 0
            ;;
        401)
            warn "Authentication failed. Check your token in ${CONFIG_DEST}"
            return 1
            ;;
        500)
            local err
            err=$(python3 -c \
                "import json,sys; d=json.load(sys.stdin); print(d.get('error','AI error'))" \
                <<< "$body" 2>/dev/null) || err="Internal error"
            echo
            warn "AI error: ${err}"
            warn "Check your API key in ${CONFIG_DEST} or type /setup-model."
            echo
            return 1
            ;;
        *)
            warn "Gateway unreachable (HTTP ${http_code:-no response})."
            warn "Check logs: ${LOG_FILE}"
            return 1
            ;;
    esac
}

# ─── 13. First-run greeting + auto-start offer ───────────────────────────────
first_run_greeting() {
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
    echo -e "${BOLD}  Testing your AI connection...${RESET}"
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
    echo

    # Wait for gateway
    local i
    for i in $(seq 1 8); do
        curl -sf "http://127.0.0.1:${GATEWAY_PORT}/health" &>/dev/null && break
        sleep 1
    done

    # Send automatic first message
    if send_chat_message "Hi! I just installed you. Introduce yourself briefly and tell me what you can do."; then
        success "AI connection works — ${PROVIDER_NAME} is responding."
    else
        warn "Could not reach the AI. You can fix settings later in ${CONFIG_DEST}"
        warn "Or type /setup-model in the terminal to reconfigure."
    fi
    echo

    # ── Offer auto-start on boot ──────────────────────────────────────────────
    echo -e "${BOLD}  Auto-start SmartopolAI when your computer boots?${RESET}"
    echo -e "  This installs a background service so SmartopolAI is always ready."
    echo
    local autostart_yn=""
    echo -ne "  Enable auto-start? [Y/n]: "
    read -r autostart_yn
    autostart_yn="${autostart_yn:-Y}"

    if [[ "$autostart_yn" =~ ^[Yy] ]]; then
        install_autostart
    else
        info "Skipped. Start manually anytime: ${CYAN}${SKYNET_DIR}/${BINARY_NAME}${RESET}"
    fi
    echo

    # Remove first-run marker
    rm -f "$SKYNET_DIR/.first-run"
}

# ─── 14. Install auto-start (launchd on macOS, systemd on Linux) ─────────────
install_autostart() {
    case "$OS" in
        Darwin)
            local plist_dir="$HOME/Library/LaunchAgents"
            local plist_file="$plist_dir/ai.smartopol.gateway.plist"
            mkdir -p "$plist_dir"
            cat > "$plist_file" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>ai.smartopol.gateway</string>
    <key>ProgramArguments</key>
    <array>
        <string>${SKYNET_DIR}/${BINARY_NAME}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>${LOG_FILE}</string>
    <key>StandardErrorPath</key>
    <string>${LOG_FILE}</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>SKYNET_CONFIG</key>
        <string>${CONFIG_DEST}</string>
    </dict>
</dict>
</plist>
PLIST
            launchctl load "$plist_file" 2>/dev/null
            success "Auto-start installed (launchd)"
            info "SmartopolAI will start automatically on login."
            info "Manage: launchctl load/unload ${plist_file}"
            ;;
        Linux)
            local service_dir="$HOME/.config/systemd/user"
            local service_file="$service_dir/smartopol-gateway.service"
            mkdir -p "$service_dir"
            cat > "$service_file" <<UNIT
[Unit]
Description=SmartopolAI Gateway
After=network.target

[Service]
Type=simple
ExecStart=${SKYNET_DIR}/${BINARY_NAME}
Environment=SKYNET_CONFIG=${CONFIG_DEST}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
UNIT
            systemctl --user daemon-reload
            systemctl --user enable smartopol-gateway.service
            systemctl --user start smartopol-gateway.service
            success "Auto-start installed (systemd user service)"
            info "Manage: systemctl --user start/stop/status smartopol-gateway"
            ;;
    esac
}

# ─── 15. Terminal REPL ────────────────────────────────────────────────────────
repl_chat() {
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
    echo -e "${BOLD}  SmartopolAI Terminal${RESET}"
    echo -e "  Type your message and press Enter."
    echo -e "  ${CYAN}/setup-model${RESET} — switch AI provider  ·  ${CYAN}/exit${RESET} — quit"
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
    echo

    # Wait until gateway is ready
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

        send_chat_message "$user_input"
    done
}

# ─── Detect existing config and ask user what to do ──────────────────────────
check_existing_config() {
    if [[ -f "$CONFIG_DEST" ]]; then
        echo -e "${BOLD}  Existing SmartopolAI installation detected:${RESET}"
        echo
        echo -e "  Config:  ${CYAN}${CONFIG_DEST}${RESET}"
        [[ -f "$SOUL_DEST" ]] && echo -e "  SOUL:    ${CYAN}${SOUL_DEST}${RESET}"

        # Show current provider
        local current_provider=""
        current_provider=$(grep -E "^provider\s*=" "$CONFIG_DEST" 2>/dev/null | cut -d'"' -f2) || true
        [[ -n "$current_provider" ]] && echo -e "  Provider: ${BOLD}${current_provider}${RESET}"

        echo
        echo -e "    1) ${BOLD}Keep existing config${RESET} — just rebuild and start"
        echo -e "    2) ${BOLD}Reconfigure${RESET} — start fresh wizard"
        echo
        local existing_choice=""
        prompt existing_choice "Choice" "1"
        echo

        if [[ "$existing_choice" == "1" ]]; then
            # Load existing values for REPL (auth token, port, provider)
            AUTH_TOKEN=$(grep 'token = ' "$CONFIG_DEST" 2>/dev/null | head -1 | cut -d'"' -f2) || true
            GATEWAY_PORT=$(grep 'port\s*=' "$CONFIG_DEST" 2>/dev/null | head -1 | sed 's/[^0-9]//g') || true
            GATEWAY_PORT="${GATEWAY_PORT:-18789}"
            PROVIDER_NAME="$current_provider"
            AGENT_MODEL=$(grep 'model\s*=' "$CONFIG_DEST" 2>/dev/null | head -1 | cut -d'"' -f2) || true
            return 0  # skip wizard
        fi
    fi
    return 1  # run wizard
}

# ─── Main ─────────────────────────────────────────────────────────────────────
main() {
    print_banner
    detect_os
    check_dependencies
    build_binary
    create_skynet_dir

    if ! check_existing_config; then
        wizard
        write_config
    fi

    health_check
    mark_first_run
    print_summary
    launch_agent
    first_run_greeting
    repl_chat
}

main "$@"
