#!/usr/bin/env bash
# One-command macOS build for Seqr.
#
# Installs prerequisites via Homebrew (Node, pnpm, Rust if missing), then builds the
# native app bundle and .dmg. Run from anywhere inside the repo:
#   ./scripts/setup-macos.sh
#
# Output: apps/desktop/src-tauri/target/release/bundle/{dmg,macos}/
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO/apps/desktop"

echo "==> Checking prerequisites"
if ! command -v brew >/dev/null 2>&1; then
    echo "Homebrew is required: https://brew.sh" >&2
    exit 1
fi
command -v node >/dev/null 2>&1 || brew install node
command -v pnpm >/dev/null 2>&1 || brew install pnpm

# Make an existing rustup install visible in this (possibly fresh) shell, so we don't
# needlessly reinstall it.
# shellcheck disable=SC1091
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
if command -v cargo >/dev/null 2>&1; then
    echo "==> Rust present: $(cargo --version)"
else
    echo "==> Installing Rust (not found)"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    # shellcheck disable=SC1091
    source "$HOME/.cargo/env"
fi

echo "==> Installing frontend dependencies"
pnpm install

echo "==> Building Seqr (this compiles the Rust core; first build is slow)"
pnpm tauri build

echo ""
echo "==> Done. Installer(s):"
find src-tauri/target/release/bundle -maxdepth 2 -name "*.dmg" -o -name "*.app" 2>/dev/null | sed 's/^/    /'
echo ""
echo "Note: the build is unsigned. On first launch, right-click the app and choose"
echo "      'Open' (or System Settings -> Privacy & Security -> Open Anyway)."
