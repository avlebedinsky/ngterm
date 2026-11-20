#!/usr/bin/env bash
# Install the Rust toolchain that rust-toolchain.toml pins, make it the default and uninstall every other toolchain.
# A CI job runs it before Swatinem/rust-cache: rust-cache puts the rustc of each installed toolchain into its key, so
# a runner image with a newer stable beside the pin would give a different key and miss the cache
# (docs/development.md, CI caches). It runs on Linux, macOS and Windows (Git Bash).
#
# Usage: scripts/ci-rust-toolchain.sh
set -euo pipefail

cd "$(dirname "$0")/.."
rustup toolchain install
pinned="$(rustup show active-toolchain | tr -d '\r' | cut -d' ' -f1)"
if [[ -z "$pinned" ]]; then
  echo "ci-rust-toolchain.sh: rustup shows no active toolchain" >&2
  exit 1
fi
rustup default "$pinned"
while IFS= read -r toolchain; do
  if [[ -n "$toolchain" && "$toolchain" != "$pinned" ]]; then
    rustup toolchain uninstall "$toolchain"
  fi
done < <(rustup toolchain list --quiet | tr -d '\r')
left="$(rustup toolchain list --quiet | tr -d '\r')"
if [[ "$left" != "$pinned" ]]; then
  echo "ci-rust-toolchain.sh: toolchains left beside $pinned: ${left//$'\n'/ }" >&2
  exit 1
fi
echo "the only toolchain: $pinned"
