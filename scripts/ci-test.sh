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

if ! grep -q "IDT: 256 entries" "$LOG_FILE"; then
  echo "FAIL: kernel did not report loading a full 256-entry IDT"
  status=1
fi

if ! grep -q "Exceptions: #0 divide error" "$LOG_FILE"; then
  echo "FAIL: kernel did not report installing its CPU exception handlers"
  status=1
fi

if ! grep -q "Double fault: task gate" "$LOG_FILE"; then
  echo "FAIL: kernel did not report a task gate for the double-fault handler"
  status=1
fi

if ! grep -q "PIC: IRQ0-15 -> vectors 32-47" "$LOG_FILE"; then
  echo "FAIL: kernel did not report remapping the PIC above the exception vectors"
  status=1
fi

# An unmasked line with no handler registered for its vector would fault as
# soon as interrupts are enabled, so the remap must not leave any enabled.
if ! grep -q "IMR 0xff/0xff (all masked)" "$LOG_FILE"; then
  echo "FAIL: PIC came out of the remap with IRQ lines unmasked"
  status=1
fi

# Every vector the exception handlers don't own must have the catch-all
# installed, or a stray interrupt escalates to a double fault.
if ! grep -q "Interrupts: catch-all on 252 spare vectors" "$LOG_FILE"; then
  echo "FAIL: kernel left vectors without a handler before enabling interrupts"
  status=1
fi

# The one line printed after `sti`, so its presence is also proof the kernel
# survived enabling interrupts.
if ! grep -q "Interrupts: enabled (IF=1)" "$LOG_FILE"; then
  echo "FAIL: kernel did not report interrupts enabled"
  status=1
fi

# Nothing should be delivered while every IRQ line is masked.
if ! grep -q "Interrupts: enabled (IF=1), IMR 0xff/0xff, 0 unhandled" "$LOG_FILE"; then
  echo "FAIL: an unexpected interrupt arrived, or the PIC masks did not hold"
  status=1
fi

# A stray interrupt on a vector nothing owns, once interrupts are on.
if grep -q "UNHANDLED INTERRUPT" "$LOG_FILE"; then
  echo "FAIL: kernel took an interrupt it had no handler for"
  status=1
fi

# The kernel halts rather than exiting, so it should print its banner exactly
# once. Twice means the machine reset and booted again - which is what a
# triple fault looks like from out here, and the failure mode enabling
# interrupts could plausibly introduce.
boots="$(grep -c "GOATos booted successfully" "$LOG_FILE")"
if [ "$boots" -gt 1 ]; then
  echo "FAIL: kernel booted $boots times - it reset instead of idling"
  status=1
fi

# `tr=0x00` would mean no TSS is loaded, so a double fault would have nowhere
# to save the interrupted registers and would triple-fault the machine.
if grep -q "tr=0x00" "$LOG_FILE"; then
  echo "FAIL: task register is not pointing at the kernel's TSS"
  status=1
fi

if grep -q "KERNEL PANIC" "$LOG_FILE"; then
  echo "FAIL: kernel panicked"
  status=1
fi

# A normal boot must not fault. This only catches an exception the handlers
# installed so far actually cover, but that is exactly the set a plain boot
# could plausibly hit.
if grep -q "CPU EXCEPTION" "$LOG_FILE"; then
  echo "FAIL: kernel took an unexpected CPU exception"
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
