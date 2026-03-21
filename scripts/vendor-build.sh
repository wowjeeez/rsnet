#!/usr/bin/env bash
set -euo pipefail

# builds libtailscale.a + bindings for the current platform
# output goes to vendored/<os>-<arch>/

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIBTAILSCALE="$ROOT/libtailscale"

OS="$(go env GOOS)"
ARCH="$(go env GOARCH)"
OUT="$ROOT/vendored/${OS}-${ARCH}"

mkdir -p "$OUT"

echo "building libtailscale.a for ${OS}-${ARCH}..."
cd "$LIBTAILSCALE"
go build -buildmode=c-archive -o "$OUT/libtailscale.a" .

echo "generating rust bindings..."
bindgen "$LIBTAILSCALE/tailscale.h" > "$OUT/libtailscale.rs"

echo "done: $OUT"
ls -lh "$OUT/libtailscale.a" "$OUT/libtailscale.rs"
