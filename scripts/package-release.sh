#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

VERSION="${1:-dev}"
TARGET="${2:-}"

if [[ -n "$TARGET" ]]; then
  cargo build --release -p patina-cli --target "$TARGET"
  BIN_PATH="target/$TARGET/release/patina"
  PKG_SUFFIX="$TARGET"
else
  cargo build --release -p patina-cli
  BIN_PATH="target/release/patina"
  PKG_SUFFIX="$(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m | tr '[:upper:]' '[:lower:]')"
fi

PKG_DIR="patina-bot-${VERSION}-${PKG_SUFFIX}"
OUT_DIR="dist"

mkdir -p "$OUT_DIR"
rm -rf "$OUT_DIR/$PKG_DIR"
mkdir -p "$OUT_DIR/$PKG_DIR"

cp "$BIN_PATH" "$OUT_DIR/$PKG_DIR/patina"
cp README.md "$OUT_DIR/$PKG_DIR/README.md"
cp config.example.json "$OUT_DIR/$PKG_DIR/config.example.json"

(
  cd "$OUT_DIR"
  tar -czf "${PKG_DIR}.tar.gz" "$PKG_DIR"
  shasum -a 256 "${PKG_DIR}.tar.gz" > "${PKG_DIR}.sha256"
)

echo "Created:"
echo "  $OUT_DIR/${PKG_DIR}.tar.gz"
echo "  $OUT_DIR/${PKG_DIR}.sha256"
