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

# The BIOS memory map the boot sector collects in real mode and hands over at
# a fixed address. Nothing consumes it yet, so these checks are the only thing
# keeping that handoff honest.
if grep -q "MEMORY MAP UNAVAILABLE" "$LOG_FILE"; then
  echo "FAIL: the boot sector handed the kernel no BIOS memory map"
  status=1
fi

if grep -q "(TRUNCATED" "$LOG_FILE"; then
  echo "FAIL: the BIOS reported more memory regions than the handoff block holds"
  status=1
fi

# Low memory is the one region every PC has and every BIOS reports, so its
# absence means the entries themselves are being misread even if the count
# looks plausible.
if ! grep -qE "^E820: +0x0000000000-0x[0-9a-f]+ +[0-9]+ KiB +usable$" "$LOG_FILE"; then
  echo "FAIL: kernel did not report conventional low memory as usable"
  status=1
fi

memory_summary="$(grep -m1 '^Memory: ' "$LOG_FILE")"
if [ -z "$memory_summary" ]; then
  echo "FAIL: kernel did not report a memory map summary"
  status=1
else
  regions="$(sed -n 's/^Memory: \([0-9]\+\) E820 regions.*/\1/p' <<<"$memory_summary")"
  usable_mib="$(sed -n 's/^Memory: .*, \([0-9]\+\) MiB usable.*/\1/p' <<<"$memory_summary")"

  # Any PC splits its address space into at least "low RAM / the hole below
  # 1MiB / RAM above it", so a map of one or two regions is a parsing bug.
  if [ -z "$regions" ] || [ "$regions" -lt 3 ]; then
    echo "FAIL: implausibly short memory map (${regions:-no} regions)"
    status=1
  fi

  # QEMU is started above with no -m, i.e. its 128MiB default; a few MiB of
  # that is reserved for the firmware and the PCI hole, so the usable total
  # lands just under it. Anything outside this window means the kernel is
  # reading lengths, not just reporting them, wrongly.
  if [ -z "$usable_mib" ] || [ "$usable_mib" -lt 100 ] || [ "$usable_mib" -gt 128 ]; then
    echo "FAIL: usable memory (${usable_mib:-none} MiB) does not match QEMU's 128MiB default"
    status=1
  fi
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
