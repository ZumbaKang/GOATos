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

# Graphics mode (roadmap 5.1): the bootloader left Mode 13h and the kernel
# filled the framebuffer. Serial is the only automated proof; a QEMU
# screendump / v86 canvas check is still needed for the solid-color claim.
if ! grep -qE "Graphics: VGA mode 0x13 320x200 @ 0x000a0000, fill color [0-9]+ \(solid\)" "$LOG_FILE"; then
  echo "FAIL: kernel did not report VGA Mode 13h graphics fill"
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

# The frame allocator built on top of that map. Its own self-test is the
# kernel's word for it; the checks after it are this script's, made against
# the addresses in the log rather than against the verdict.
if ! grep -q "Frames: self-test ok" "$LOG_FILE"; then
  echo "FAIL: the frame allocator's boot self-test did not pass"
  status=1
fi

if grep -q "SELF-TEST FAILED" "$LOG_FILE"; then
  echo "FAIL: the frame allocator's boot self-test reported a failure"
  status=1
fi

if grep -q "NO ALLOCATABLE MEMORY" "$LOG_FILE"; then
  echo "FAIL: no usable memory survived the frame allocator's reserved ranges"
  status=1
fi

# The kernel must exclude its own image, and it must work that out from the
# linker rather than from a guess: an under-sized reservation would leave the
# allocator handing out frames the kernel is running in.
if ! grep -qE "^Frames: reserved 0x00010000-0x[0-9a-f]+ +[0-9]+ frames  kernel image" "$LOG_FILE"; then
  echo "FAIL: the frame allocator did not reserve the kernel image"
  status=1
fi

frame_summary="$(grep -m1 '^Frames: [0-9]' "$LOG_FILE")"
frame_mib="$(sed -n 's/^Frames: [0-9]\+ x 4 KiB allocatable (\([0-9]\+\) MiB).*/\1/p' <<<"$frame_summary")"
# The pool is the usable memory checked above, less the few hundred KiB of
# reservations, so it lands in the same window. Far below it would mean whole
# regions are being dropped; above it, that the reservations are not applied.
if [ -z "$frame_mib" ] || [ "$frame_mib" -lt 100 ] || [ "$frame_mib" -gt 128 ]; then
  echo "FAIL: frame pool (${frame_mib:-none} MiB) does not match the usable memory reported"
  status=1
fi

mapfile -t reserved_ranges < <(sed -n 's/^Frames: reserved 0x\([0-9a-f]\+\)-0x\([0-9a-f]\+\).*/\1 \2/p' "$LOG_FILE")
mapfile -t allocated_frames < <(sed -n 's/^Frame: *#[0-9]\+ at 0x\([0-9a-f]\+\)$/\1/p' "$LOG_FILE")

if [ "${#reserved_ranges[@]}" -lt 3 ]; then
  echo "FAIL: kernel reported ${#reserved_ranges[@]} reserved ranges, expected at least 3"
  status=1
fi

if [ "${#allocated_frames[@]}" -lt 4 ]; then
  echo "FAIL: kernel printed ${#allocated_frames[@]} allocated frame addresses, expected at least 4"
  status=1
fi

# Same-sized frames at distinct addresses cannot overlap, so distinctness is
# the whole of "no overlaps" here.
distinct="$(printf '%s\n' "${allocated_frames[@]}" | sort -u | wc -l)"
if [ "$distinct" -ne "${#allocated_frames[@]}" ]; then
  echo "FAIL: allocator handed out the same frame twice"
  status=1
fi

for frame_hex in "${allocated_frames[@]}"; do
  frame_addr="$((16#$frame_hex))"
  if [ "$((frame_addr % 4096))" -ne 0 ]; then
    echo "FAIL: allocated frame 0x$frame_hex is not 4KiB-aligned"
    status=1
  fi
  for range in "${reserved_ranges[@]}"; do
    # shellcheck disable=SC2086 # deliberate word splitting: "start end"
    set -- $range
    if [ "$frame_addr" -ge "$((16#$1))" ] && [ "$frame_addr" -lt "$((16#$2))" ]; then
      echo "FAIL: allocated frame 0x$frame_hex is inside reserved range 0x$1-0x$2"
      status=1
    fi
  done
done

# Paging: identity map + CR0.PG. Surviving past this line (the boot-success
# message still appears once) is the real proof the map covers the kernel;
# these checks pin the banner shape and the size to QEMU's RAM.
if ! grep -q "Paging: identity-mapped" "$LOG_FILE"; then
  echo "FAIL: kernel did not report enabling an identity-mapped page table"
  status=1
fi

if grep -q "Paging: FAILED" "$LOG_FILE"; then
  echo "FAIL: paging setup reported failure"
  status=1
fi

if ! grep -qE "Paging: identity-mapped 0x00000000-0x[0-9a-f]+ \([0-9]+ MiB\) via [0-9]+ page tables, CR3=0x[0-9a-f]+, PG=1" "$LOG_FILE"; then
  echo "FAIL: paging banner missing CR3/PG=1 (paging not actually enabled)"
  status=1
fi

paging_summary="$(grep -m1 '^Paging: identity-mapped' "$LOG_FILE")"
paging_mib="$(sed -n 's/^Paging: identity-mapped 0x[0-9a-f]\+-0x[0-9a-f]\+ (\([0-9]\+\) MiB).*/\1/p' <<<"$paging_summary")"
paging_tables="$(sed -n 's/^Paging:.*via \([0-9]\+\) page tables.*/\1/p' <<<"$paging_summary")"
# Identity map covers usable RAM rounded up to 4 MiB, so under QEMU's 128MiB
# default it is exactly 128 MiB via 32 page tables. Far below that would mean
# only low memory was mapped (and the kernel image above 1MiB would fault).
if [ -z "$paging_mib" ] || [ "$paging_mib" -lt 100 ] || [ "$paging_mib" -gt 128 ]; then
  echo "FAIL: identity-mapped window (${paging_mib:-none} MiB) does not match QEMU's 128MiB default"
  status=1
fi
if [ -z "$paging_tables" ] || [ "$paging_tables" -ne $((paging_mib / 4)) ]; then
  echo "FAIL: page table count (${paging_tables:-none}) does not match mapped size (${paging_mib:-?} MiB / 4)"
  status=1
fi

# Kernel heap + global allocator. The self-test is the proof `alloc::vec::Vec`
# works; the size/range checks keep the reservation honest.
if ! grep -q "Heap: self-test ok" "$LOG_FILE"; then
  echo "FAIL: the heap allocator's boot self-test did not pass"
  status=1
fi

if grep -q "Heap: SELF-TEST FAILED" "$LOG_FILE"; then
  echo "FAIL: the heap allocator's boot self-test reported a failure"
  status=1
fi

if grep -q "Heap: FAILED" "$LOG_FILE"; then
  echo "FAIL: kernel could not reserve a contiguous heap region"
  status=1
fi

if ! grep -qE "Heap: 0x[0-9a-f]+-0x[0-9a-f]+ \(1024 KiB\), free-list allocator ready" "$LOG_FILE"; then
  echo "FAIL: heap banner missing the expected 1 MiB free-list region"
  status=1
fi

# Heap vs stack layout, including the unmapped guard page between the DF
# stack and the kernel stack. The kernel's own check is necessary but not
# sufficient on its own (a broken predicate can bless itself); re-parse the
# absolute ranges from the log and re-derive adjacency/disjointness here.
if ! grep -q "Paging: stack guard page" "$LOG_FILE"; then
  echo "FAIL: kernel did not report an unmapped stack guard page"
  status=1
fi

if ! grep -q "Layout: DF stack" "$LOG_FILE"; then
  echo "FAIL: kernel did not report the heap/stack/guard layout"
  status=1
fi

if grep -q "Layout: OVERLAP/MISMATCH" "$LOG_FILE"; then
  echo "FAIL: kernel reported a heap/stack layout overlap or size mismatch"
  status=1
fi

if ! grep -qE "Layout: DF stack 0x[0-9a-f]+-0x[0-9a-f]+ \(4 KiB\), guard 0x[0-9a-f]+-0x[0-9a-f]+ \(4 KiB, unmapped\), kernel stack 0x[0-9a-f]+-0x[0-9a-f]+ \(64 KiB\), heap 0x[0-9a-f]+-0x[0-9a-f]+ \(1024 KiB\) - ok" "$LOG_FILE"; then
  echo "FAIL: layout banner missing the expected stack/guard/heap sizes or ok verdict"
  status=1
fi

layout_image="$(sed -n 's/^Layout: image 0x\([0-9a-f]\+\)-0x\([0-9a-f]\+\)$/\1 \2/p' "$LOG_FILE" | head -1)"
layout_dfstack="$(sed -n 's/^Layout: dfstack 0x\([0-9a-f]\+\)-0x\([0-9a-f]\+\)$/\1 \2/p' "$LOG_FILE" | head -1)"
layout_guard="$(sed -n 's/^Layout: guard 0x\([0-9a-f]\+\)-0x\([0-9a-f]\+\)$/\1 \2/p' "$LOG_FILE" | head -1)"
layout_kstack="$(sed -n 's/^Layout: kstack 0x\([0-9a-f]\+\)-0x\([0-9a-f]\+\)$/\1 \2/p' "$LOG_FILE" | head -1)"
layout_heap="$(sed -n 's/^Layout: heap 0x\([0-9a-f]\+\)-0x\([0-9a-f]\+\)$/\1 \2/p' "$LOG_FILE" | head -1)"

if [ -z "$layout_image" ] || [ -z "$layout_dfstack" ] || [ -z "$layout_guard" ] || [ -z "$layout_kstack" ] || [ -z "$layout_heap" ]; then
  echo "FAIL: kernel did not print absolute layout ranges for image/dfstack/guard/kstack/heap"
  status=1
else
  # shellcheck disable=SC2086
  set -- $layout_image; image_lo=$((16#$1)); image_hi=$((16#$2))
  # shellcheck disable=SC2086
  set -- $layout_dfstack; dfstack_lo=$((16#$1)); dfstack_hi=$((16#$2))
  # shellcheck disable=SC2086
  set -- $layout_guard; guard_lo=$((16#$1)); guard_hi=$((16#$2))
  # shellcheck disable=SC2086
  set -- $layout_kstack; kstack_lo=$((16#$1)); kstack_hi=$((16#$2))
  # shellcheck disable=SC2086
  set -- $layout_heap; heap_lo=$((16#$1)); heap_hi=$((16#$2))

  if [ "$((kstack_hi - kstack_lo))" -ne $((64 * 1024)) ]; then
    echo "FAIL: kernel stack size is $((kstack_hi - kstack_lo)) bytes, expected 64 KiB"
    status=1
  fi
  if [ "$((dfstack_hi - dfstack_lo))" -ne 4096 ]; then
    echo "FAIL: double-fault stack size is $((dfstack_hi - dfstack_lo)) bytes, expected 4 KiB"
    status=1
  fi
  if [ "$((guard_hi - guard_lo))" -ne 4096 ]; then
    echo "FAIL: guard page size is $((guard_hi - guard_lo)) bytes, expected 4 KiB"
    status=1
  fi
  if [ "$((heap_hi - heap_lo))" -ne $((1024 * 1024)) ]; then
    echo "FAIL: heap size is $((heap_hi - heap_lo)) bytes, expected 1 MiB"
    status=1
  fi
  if [ "$kstack_lo" -lt "$image_lo" ] || [ "$kstack_hi" -gt "$image_hi" ]; then
    echo "FAIL: kernel stack is outside the kernel image"
    status=1
  fi
  if [ "$dfstack_lo" -lt "$image_lo" ] || [ "$dfstack_hi" -gt "$image_hi" ]; then
    echo "FAIL: double-fault stack is outside the kernel image"
    status=1
  fi
  if [ "$guard_lo" -lt "$image_lo" ] || [ "$guard_hi" -gt "$image_hi" ]; then
    echo "FAIL: guard page is outside the kernel image"
    status=1
  fi
  # entry.s order: DF stack | guard | kernel stack.
  if [ "$dfstack_hi" -ne "$guard_lo" ]; then
    echo "FAIL: double-fault stack is not immediately below the guard page"
    status=1
  fi
  if [ "$guard_hi" -ne "$kstack_lo" ]; then
    echo "FAIL: guard page is not immediately below the kernel stack"
    status=1
  fi
  # Half-open ranges overlap when a_lo < b_hi && b_lo < a_hi.
  if [ "$kstack_lo" -lt "$dfstack_hi" ] && [ "$dfstack_lo" -lt "$kstack_hi" ]; then
    echo "FAIL: kernel stack and double-fault stack overlap"
    status=1
  fi
  if [ "$image_lo" -lt "$heap_hi" ] && [ "$heap_lo" -lt "$image_hi" ]; then
    echo "FAIL: heap overlaps the kernel image (and therefore a stack)"
    status=1
  fi
fi

# PIT (IRQ0): programmed, handler installed, line unmasked. The banner's IMR
# must show IRQ0 clear on the master (0xfe) with the slave still fully masked.
if ! grep -q "PIT: channel 0 at 100 Hz" "$LOG_FILE"; then
  echo "FAIL: kernel did not report programming the PIT"
  status=1
fi

if ! grep -qE "PIT: channel 0 at 100 Hz \(IRQ0 -> vector 32, divisor [0-9]+\), IMR 0xfe/0xff" "$LOG_FILE"; then
  echo "FAIL: PIT banner missing IRQ0 vector/IMR (timer IRQ not actually unmasked)"
  status=1
fi

# Once-a-second tick reports from the idle loop - the proof IRQ0 is firing.
# Need at least two, strictly increasing, or the counter is stuck / not running.
mapfile -t pit_ticks < <(sed -n 's/^PIT: tick \([0-9]\+\) ([0-9]\+ s)$/\1/p' "$LOG_FILE")
if [ "${#pit_ticks[@]}" -lt 2 ]; then
  echo "FAIL: kernel printed ${#pit_ticks[@]} PIT tick report(s), expected at least 2"
  status=1
else
  prev="${pit_ticks[0]}"
  for tick in "${pit_ticks[@]:1}"; do
    if [ "$tick" -le "$prev" ]; then
      echo "FAIL: PIT tick counter did not increase ($prev -> $tick)"
      status=1
      break
    fi
    prev="$tick"
  done
fi

# PS/2 keyboard (IRQ1): handler installed, line unmasked. Master IMR should
# now have IRQ0 and IRQ1 clear (0xfc) with the slave still fully masked.
if ! grep -q "Keyboard: PS/2 on IRQ1 -> vector 33" "$LOG_FILE"; then
  echo "FAIL: kernel did not report installing the PS/2 keyboard driver"
  status=1
fi

if ! grep -qE "Keyboard: PS/2 on IRQ1 -> vector 33, IMR 0xfc/0xff" "$LOG_FILE"; then
  echo "FAIL: keyboard banner missing IRQ1 vector/IMR (keyboard IRQ not actually unmasked)"
  status=1
fi

# Input event queue (roadmap 3.3): IRQ1 pushes, the idle loop drains. The
# capacity line is proof the queue was wired up before the idle loop starts.
if ! grep -qE "Input: [0-9]+-event ring buffer \(IRQ pushes, idle loop drains\)" "$LOG_FILE"; then
  echo "FAIL: kernel did not report the input event queue"
  status=1
fi

# Shell (roadmap 4.1 / 4.2): line editor + built-in command dispatcher.
if ! grep -qE "Shell: line editor \([0-9]+ chars\) \+ builtins \(help/clear/echo/about\)" "$LOG_FILE"; then
  echo "FAIL: kernel did not report the shell line editor / builtins"
  status=1
fi

# Round-robin scheduler (roadmap 4.4): shell + two demo tasks on a FIFO
# ready queue, still with explicit yields (no preemption yet).
if ! grep -qE "Tasks: round-robin ready-queue \([0-9]+ tasks, shell \+ demo-a [0-9]+ \+ demo-b [0-9]+\)" "$LOG_FILE"; then
  echo "FAIL: kernel did not report the round-robin ready-queue scheduler"
  status=1
fi

# Both demo tasks must actually run: at least two increasing counter lines
# each, interleaved with the shell's PIT reports.
for label in 1 2; do
  mapfile -t demo_counts < <(sed -n "s/^Task: demo-${label} counter \([0-9]\+\) (turns [0-9]\+)$/\1/p" "$LOG_FILE")
  if [ "${#demo_counts[@]}" -lt 2 ]; then
    echo "FAIL: kernel printed ${#demo_counts[@]} demo-${label} counter report(s), expected at least 2"
    status=1
  else
    prev="${demo_counts[0]}"
    for n in "${demo_counts[@]:1}"; do
      if [ "$n" -le "$prev" ]; then
        echo "FAIL: demo-${label} counter did not increase ($prev -> $n)"
        status=1
        break
      fi
      prev="$n"
    done
  fi
done

# Fairness: the last Scheduler turns line should show three task counts
# within one of each other (equal turns aside from off-by-one at the
# sampling boundary).
mapfile -t turn_lines < <(grep -E '^Scheduler: turns \[0\]=[0-9]+ \[1\]=[0-9]+ \[2\]=[0-9]+$' "$LOG_FILE")
if [ "${#turn_lines[@]}" -lt 1 ]; then
  echo "FAIL: kernel did not report scheduler turn counts"
  status=1
else
  last_turns="${turn_lines[-1]}"
  t0="$(sed -n 's/^Scheduler: turns \[0\]=\([0-9]\+\) \[1\]=\([0-9]\+\) \[2\]=\([0-9]\+\)$/\1/p' <<<"$last_turns")"
  t1="$(sed -n 's/^Scheduler: turns \[0\]=\([0-9]\+\) \[1\]=\([0-9]\+\) \[2\]=\([0-9]\+\)$/\2/p' <<<"$last_turns")"
  t2="$(sed -n 's/^Scheduler: turns \[0\]=\([0-9]\+\) \[1\]=\([0-9]\+\) \[2\]=\([0-9]\+\)$/\3/p' <<<"$last_turns")"
  if [ -z "$t0" ] || [ -z "$t1" ] || [ -z "$t2" ]; then
    echo "FAIL: could not parse scheduler turn counts from: $last_turns"
    status=1
  elif [ "$t0" -lt 2 ] || [ "$t1" -lt 2 ] || [ "$t2" -lt 2 ]; then
    echo "FAIL: scheduler turn counts too low for a fairness check ($last_turns)"
    status=1
  else
    max="$t0"
    min="$t0"
    for t in "$t1" "$t2"; do
      if [ "$t" -gt "$max" ]; then max="$t"; fi
      if [ "$t" -lt "$min" ]; then min="$t"; fi
    done
    spread=$((max - min))
    if [ "$spread" -gt 1 ]; then
      echo "FAIL: round-robin turns not fair (spread $spread): $last_turns"
      status=1
    fi
  fi
fi

# Proof all three tasks produced output.
if ! grep -q "PIT: tick" "$LOG_FILE" \
  || ! grep -q "Task: demo-1 counter" "$LOG_FILE" \
  || ! grep -q "Task: demo-2 counter" "$LOG_FILE"; then
  echo "FAIL: round-robin tasks did not all produce output"
  status=1
fi

# A stray interrupt on a vector nothing owns, once interrupts are on.
# (IRQ0/IRQ1 have real handlers now, so timer/keyboard must not land here.)
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
