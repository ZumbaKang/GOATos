#!/usr/bin/env bash
# Assembles a static site that boots GOATos in the browser via v86:
#   1. builds the GRUB ISO the same way `make iso` does for local/QEMU use
#   2. fetches the v86 emulator runtime (wasm + JS + BIOS blobs) via npm
#   3. copies everything into $OUT_DIR, ready to be served as-is (e.g. by
#      GitHub Pages, or locally with `python3 -m http.server`)
#
# v86's runtime files are fetched here rather than committed to the repo, so
# multi-megabyte binary blobs don't bloat git history and stay in sync with
# whatever version of v86 is current.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${OUT_DIR:-$ROOT_DIR/_site}"
V86_SCRATCH_DIR="$(mktemp -d)"
trap 'rm -rf "$V86_SCRATCH_DIR"' EXIT

# The npm package only ships the JS/WASM runtime, not the BIOS blobs, so
# those are fetched straight from the v86 repo, pinned to a tag for stability.
V86_BIOS_REF="${V86_BIOS_REF:-master}"
V86_BIOS_BASE_URL="https://raw.githubusercontent.com/copy/v86/${V86_BIOS_REF}/bios"

echo "==> Building the GOATos disk image"
make -C "$ROOT_DIR" disk

echo "==> Fetching the v86 emulator runtime"
(cd "$V86_SCRATCH_DIR" && npm init -y >/dev/null 2>&1 && npm install --no-save --no-audit --no-fund v86 >/dev/null)

echo "==> Assembling static site into $OUT_DIR"
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR/v86"

cp "$ROOT_DIR/web/index.html" "$OUT_DIR/index.html"
cp "$ROOT_DIR/build/disk.img" "$OUT_DIR/disk.img"

V86_PKG="$V86_SCRATCH_DIR/node_modules/v86"
cp "$V86_PKG/build/libv86.js" "$OUT_DIR/v86/libv86.js"
cp "$V86_PKG/build/v86.wasm" "$OUT_DIR/v86/v86.wasm"
curl -sSfL "$V86_BIOS_BASE_URL/seabios.bin" -o "$OUT_DIR/v86/seabios.bin"
curl -sSfL "$V86_BIOS_BASE_URL/vgabios.bin" -o "$OUT_DIR/v86/vgabios.bin"

echo "==> Done. Serve locally with:"
echo "      python3 -m http.server -d '$OUT_DIR' 8080"
echo "    then open http://localhost:8080"
