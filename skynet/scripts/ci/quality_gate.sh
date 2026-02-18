#!/usr/bin/env bash
# quality_gate.sh — Run all CI checks locally before push.
# Usage: ./scripts/ci/quality_gate.sh [--fix]
#
# --fix  Auto-fix formatting instead of just checking.

set -euo pipefail

SKYNET_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$SKYNET_ROOT"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

FIX_MODE=false
if [[ "${1:-}" == "--fix" ]]; then
    FIX_MODE=true
fi

pass() { echo -e "${GREEN}  PASS${NC} $1"; }
fail() { echo -e "${RED}  FAIL${NC} $1"; exit 1; }
info() { echo -e "${YELLOW}  >>>>${NC} $1"; }

echo ""
echo "=== Skynet Quality Gate ==="
echo ""

# 1. Format check
if $FIX_MODE; then
    info "Formatting (auto-fix)..."
    cargo fmt --all
    pass "cargo fmt --all (applied)"
else
    info "Formatting (check)..."
    if cargo fmt --all -- --check 2>/dev/null; then
        pass "cargo fmt"
    else
        fail "cargo fmt — run './scripts/ci/quality_gate.sh --fix' to auto-fix"
    fi
fi

# 2. Clippy
info "Clippy (warnings = errors)..."
if cargo clippy --workspace --all-targets -- -D warnings 2>&1; then
    pass "cargo clippy"
else
    fail "cargo clippy — fix warnings before pushing"
fi

# 3. Tests
info "Tests..."
if cargo test --workspace 2>&1; then
    pass "cargo test"
else
    fail "cargo test — tests are failing"
fi

# 4. Build check (release mode, quick)
info "Build check..."
if cargo check --workspace 2>&1; then
    pass "cargo check"
else
    fail "cargo check — build errors detected"
fi

echo ""
echo -e "${GREEN}=== All checks passed ===${NC}"
echo ""
