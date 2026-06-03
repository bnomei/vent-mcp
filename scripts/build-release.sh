#!/usr/bin/env bash
#
# Builds the release binary for the requested Rust target.
#
# The script is intentionally small because the target and toolchain decisions
# belong to CI. It requires TARGET, prefers `cross` for musl builds when
# available, and otherwise delegates to Cargo so release automation uses one
# consistent build entry point per target.
set -euo pipefail

: "${TARGET:?TARGET is required}"

if [[ "$TARGET" == *"musl"* ]]; then
  if command -v cross >/dev/null 2>&1; then
    cross build --release --target "$TARGET"
  else
    cargo build --release --target "$TARGET"
  fi
else
  cargo build --release --target "$TARGET"
fi
