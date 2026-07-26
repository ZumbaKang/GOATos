#!/usr/bin/env bash
# Boots build/disk.img headlessly in QEMU and checks the serial output for
# proof of a successful boot (and the absence of known failure markers).
# This is GOATos's minimum viable automated test: "does the kernel actually
# boot, with no panic and no disk error" - see
# .cursor/skills/qemu-testing-and-verification/SKILL.md for the manual/
# visual verification steps (VGA screendump, web demo) this doesn't replace.
#
# Usage: scripts/ci-test.sh [timeout_seconds]
set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TIMEOUT_SECONDS="${1:-15}"
LOG_FILE="$(mktemp)"
trap 'rm -f "$LOG_FILE"' EXIT

echo "==> Building the GOATos disk image"
make -C "$ROOT_DIR" disk

echo "==> Booting build/disk.img in QEMU (headless, ${TIMEOUT_SECONDS}s timeout)"
timeout "$TIMEOUT_SECONDS" qemu-system-i386 \
  -drive file="$ROOT_DIR/build/disk.img",format=raw \
  -serial file:"$LOG_FILE" \
  -display none
# `timeout` killing QEMU (exit 124) is expected: the kernel halts in an
# infinite loop after printing, so QEMU never exits on its own.

echo "==> Serial output:"
cat "$LOG_FILE"
echo "=================="

status=0

if ! grep -q "GOATos booted successfully" "$LOG_FILE"; then
  echo "FAIL: expected boot-confirmation message not found in serial output"
  status=1
fi

if ! grep -q "GDT: kernel-owned" "$LOG_FILE"; then
  echo "FAIL: kernel did not report loading its own GDT"
  status=1
fi

if grep -q "KERNEL PANIC" "$LOG_FILE"; then
  echo "FAIL: kernel panicked"
  status=1
fi

if grep -q "disk read error" "$LOG_FILE"; then
  echo "FAIL: bootloader reported a disk read error"
  status=1
fi

if [ "$status" -eq 0 ]; then
  echo "PASS: kernel booted successfully with no panics or disk errors"
fi

exit "$status"
