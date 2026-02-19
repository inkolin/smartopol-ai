#!/usr/bin/env bash
# install.sh — SmartopolAI one-liner installer
# Usage: curl -fsSL https://raw.githubusercontent.com/inkolin/smartopol-ai/main/install.sh | bash
#
# Clones the repository then delegates to setup.sh.

set -euo pipefail

# ─── Config ───────────────────────────────────────────────────────────────────
REPO_URL="https://github.com/inkolin/smartopol-ai.git"
BRANCH="main"
DEFAULT_INSTALL_DIR="$HOME/.local/share/smartopol-ai"
INSTALL_DIR="${INSTALL_DIR:-$DEFAULT_INSTALL_DIR}"

# ─── Colours ──────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
RESET='\033[0m'

info()    { echo -e "${CYAN}  →${RESET} $*"; }
success() { echo -e "${GREEN}  ✓${RESET} $*"; }
warn()    { echo -e "${YELLOW}  !${RESET} $*"; }
die()     { echo -e "${RED}  ✗${RESET} $*" >&2; exit 1; }

# ─── Banner ───────────────────────────────────────────────────────────────────
echo
echo -e "${CYAN}  ╔═══════════════════════════════════════════╗${RESET}"
echo -e "${CYAN}  ║${RESET}   ${BOLD}SmartopolAI — One-liner Installer${RESET}      ${CYAN}║${RESET}"
echo -e "${CYAN}  ║${RESET}   github.com/inkolin/smartopol-ai        ${CYAN}║${RESET}"
echo -e "${CYAN}  ╚═══════════════════════════════════════════╝${RESET}"
echo

# ─── OS check ─────────────────────────────────────────────────────────────────
OS="$(uname -s)"
case "$OS" in
    Linux)  ;;
    Darwin) ;;
    CYGWIN*|MINGW*|MSYS*|Windows_NT)
        die "Windows is not supported natively.
  Please install WSL2: https://learn.microsoft.com/en-us/windows/wsl/install
  Then run this one-liner inside WSL2."
        ;;
    *)
        die "Unsupported OS: $OS"
        ;;
esac

# ─── Dependency check ─────────────────────────────────────────────────────────
for cmd in git curl; do
    if ! command -v "$cmd" &>/dev/null; then
        die "'$cmd' is required but not installed.
  macOS:  brew install $cmd
  Ubuntu: sudo apt install $cmd
  Fedora: sudo dnf install $cmd"
    fi
done

# ─── Clone or update ──────────────────────────────────────────────────────────
if [[ -d "$INSTALL_DIR/.git" ]]; then
    info "Repository already exists at $INSTALL_DIR — updating..."
    git -C "$INSTALL_DIR" pull --ff-only origin "$BRANCH"
    success "Repository updated"
else
    info "Cloning SmartopolAI into $INSTALL_DIR..."
    mkdir -p "$(dirname "$INSTALL_DIR")"
    git clone --branch "$BRANCH" --depth 1 "$REPO_URL" "$INSTALL_DIR"
    success "Repository cloned → $INSTALL_DIR"
fi

# ─── Delegate to setup.sh ─────────────────────────────────────────────────────
SETUP="$INSTALL_DIR/setup.sh"
if [[ ! -f "$SETUP" ]]; then
    die "setup.sh not found at $SETUP — the repository may be incomplete."
fi

chmod +x "$SETUP"
info "Launching setup wizard..."
echo

exec "$SETUP"
