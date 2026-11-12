#!/usr/bin/env bash
# Builds the Windows installer (a per-user setup .exe) on macOS or Linux.
#
# Needs, once:
#   rustup target add x86_64-pc-windows-msvc
#   cargo install --locked cargo-xwin   # fetches Microsoft's Windows SDK and C runtime on first use
#   brew install llvm nsis              # or your distribution's llvm and nsis packages
#
# The installer lands in target/x86_64-pc-windows-msvc/release/bundle/nsis/.
set -euo pipefail
cd "$(dirname "$0")/.."

# Homebrew keeps LLVM off the PATH; cargo-xwin needs its clang-cl, lld-link and llvm-rc.
if command -v brew >/dev/null 2>&1 && [ -d "$(brew --prefix)/opt/llvm/bin" ]; then
  export PATH="$(brew --prefix)/opt/llvm/bin:$PATH"
fi
# makensis aborts while writing its language tables under the plain "C" locale.
export LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8

npx tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis "$@"
