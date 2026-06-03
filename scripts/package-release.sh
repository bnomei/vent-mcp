#!/usr/bin/env bash
#
# Packages a non-Windows release binary as a tar.gz archive with a SHA-256 file.
#
# The script validates the expected target binary before packaging so release
# jobs fail loudly when a build step was skipped or targeted the wrong triple. It
# leaves Windows packaging to the PowerShell script because `.exe` archives need a
# different layout and zip format.
set -euo pipefail

: "${TARGET:?TARGET is required}"
: "${VERSION:?VERSION is required}"
BIN_NAME=${BIN_NAME:-vent}
OUT_DIR=${OUT_DIR:-dist}

mkdir -p "$OUT_DIR"

BIN_PATH="target/${TARGET}/release/${BIN_NAME}"
if [[ -f "${BIN_PATH}.exe" ]]; then
  echo "Windows binary detected; use scripts/package-release.ps1 instead." >&2
  exit 1
fi

if [[ ! -f "$BIN_PATH" ]]; then
  echo "Binary not found: $BIN_PATH" >&2
  exit 1
fi

ARCHIVE_NAME="${BIN_NAME}-v${VERSION}-${TARGET}.tar.gz"

tar -C "target/${TARGET}/release" -czf "${OUT_DIR}/${ARCHIVE_NAME}" "$BIN_NAME"

if command -v sha256sum >/dev/null 2>&1; then
  sha256sum "${OUT_DIR}/${ARCHIVE_NAME}" > "${OUT_DIR}/${ARCHIVE_NAME}.sha256"
elif command -v shasum >/dev/null 2>&1; then
  shasum -a 256 "${OUT_DIR}/${ARCHIVE_NAME}" > "${OUT_DIR}/${ARCHIVE_NAME}.sha256"
fi
