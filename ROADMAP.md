# GOATos Roadmap

The README's [Status](README.md#status) list is intentionally short - each
unchecked item there (e.g. "Physical & virtual memory management") is far
too big to do in one pass. This file breaks each of those items down into
small, specific, independently-shippable tasks, roughly in the order they
should be tackled. Each task is scoped to be doable (and verifiable) in a
single focused session, building directly on the previous one.

**How to use this file:** pick the first unchecked task, do it, verify it
per its "Done when" criteria, check it off, commit, and move to the next
one. Don't jump ahead - most tasks assume the ones above them are done.
Each phase also has a matching skill under
[`.cursor/skills/`](.cursor/skills/) with deeper technical guidance,
conventions, and gotchas - read it before starting the phase.

**This is automated end-to-end.** A [Cursor Automation](https://cursor.com/automations)
runs [`.cursor/skills/roadmap-automation/`](.cursor/skills/roadmap-automation/SKILL.md)
to pick up the next unchecked task here, implement it, and open a PR;
[`.github/workflows/ci.yml`](.github/workflows/ci.yml) builds, lints, and
boot-tests every PR and auto-merges it if that passes. You can still do a
task manually (same procedure) at any time - the automation just means
the OS keeps building itself out even when no one's driving.

---

## Phase 0 - Done

- [x] Boots via a hand-written bootloader (`boot/boot.asm`), no GRUB, no
      bootloader crate.
- [x] VGA text-mode output (`kernel/src/vga.rs`) - the "displayable"
      surface, works identically on real BIOS/QEMU and in a browser via v86.
- [x] Serial output (`kernel/src/serial.rs`) as a best-effort, headless
      testing aid.
- [x] Live browser demo (`web/`, `scripts/build-web-demo.sh`,
      `.github/workflows/deploy-pages.yml`), auto-deployed to GitHub Pages.

---

## Phase 1 - Interrupts & exceptions

*Skill: [`.cursor/skills/interrupts-and-exceptions/`](.cursor/skills/interrupts-and-exceptions/SKILL.md)*

Before this phase the kernel ran with interrupts off and no IDT of its own,
so any bug that tripped a CPU exception had undefined, unrecoverable
behavior (most likely a silent reboot). This phase turned "the kernel just
froze" into a readable diagnostic, which makes every phase after it much
faster to debug.

- [x] **1.1 - Kernel-owned GDT.** Rewrite the GDT in Rust
      (`kernel/src/gdt.rs`), loaded at kernel startup, replacing reliance on
      the minimal one `boot.asm` builds just to get into protected mode.
      *Done when:* the kernel loads its own GDT and still boots/prints
      normally in QEMU and v86.
- [x] **1.2 - IDT scaffolding.** Add `kernel/src/idt.rs`: the 256-entry IDT
      structure, a way to register a handler for a given vector, and the
      `lidt` call to load it. No real handlers yet.
      *Done when:* the IDT loads without faulting (verify by loading it and
      still reaching the existing boot-confirmation output).
- [x] **1.3 - Core exception handlers.** Add handlers for divide error (0),
      invalid opcode (6), and general protection fault (13) that print the
      vector number (and error code, where applicable) to VGA + serial,
      then halt - no recovery, just a readable crash.
      *Done when:* deliberately triggering each exception (e.g. an integer
      divide by zero behind a debug flag) shows the expected message
      instead of a silent freeze, in both QEMU and v86.
- [x] **1.4 - Double-fault handler with its own stack.** Set up a TSS with
      a dedicated stack for the double-fault handler (vector 8). Without
      this, a fault *while handling a fault* (e.g. a stack overflow)
      triple-faults the CPU and silently reboots the VM instead of printing
      anything.
      *Done when:* a deliberately-triggered stack overflow prints a
      double-fault message instead of rebooting.
      *Done as:* vector 8 goes through a task gate, so the CPU loads a whole
      register set - `esp` included - from a TSS of its own
      (`kernel/src/tss.rs`); 32-bit x86 has no equivalent of x86-64's IST,
      and a fault taken in ring 0 otherwise keeps using the stack it
      interrupted. Verified against a *real* double fault
      (`KERNEL_FEATURES=trigger-double-fault`, which unregisters the
      divide-error handler and then divides by zero, so the CPU faults while
      delivering a fault) - not against a stack overflow: without paging
      there is no guard page below the kernel stack, and segment limits, the
      only other way to bound a stack, are not enforced by QEMU's or v86's
      CPU emulation, so an overflow silently corrupts memory instead of
      faulting. Closing that gap is task 2.6 below.
- [x] **1.5 - Remap the PIC.** Reprogram the 8259 PIC so hardware IRQs land
      on vectors 32+ instead of colliding with the CPU exception vectors
      (0-31) - a classic, well-documented gotcha.
      *Done when:* the PIC is remapped and interrupts are still masked (no
      handlers yet, so nothing should fire).
      *Done as:* `kernel/src/pic.rs` runs the four-word ICW sequence on both
      cascaded 8259s to put IRQ0-15 on vectors 32-47, then writes 0xff to both
      interrupt mask registers (initialization clears them, i.e. enables every
      line, so re-masking is not optional). ICW2 is write-only, so the kernel
      banner reports the vector range it programmed alongside the masks it
      reads back; the remap itself was verified externally against QEMU's
      `info pic`, which shows `irq_base=20`/`irq_base=28` and `imr=ff` on both.
- [x] **1.6 - Enable interrupts.** Re-enable interrupts (`sti`) now that
      there's an IDT and PIC in a known state. Add a "spurious/unhandled
      interrupt" default handler for any vector without a real one yet, so
      an unexpected IRQ prints something instead of crashing.
      *Done when:* the kernel boots normally with interrupts enabled and
      idles (via `hlt`) without any unexpected crashes.
      *Done as:* `kernel/src/interrupts.rs` fills all 252 vectors the
      exception handlers don't own with a catch-all (one monomorphised entry
      point per vector, so the report can name it, and the error-code shape on
      exactly the vectors that push one), then runs `sti`. An unhandled CPU
      exception is reported and halts - resuming would re-execute the faulting
      instruction forever - while a stray IRQ or a software `int` to an unused
      vector is reported once and resumed from, since taking the machine down
      over one is worse than carrying on. That made `pic::end_of_interrupt`
      part of this task rather than of the first driver: an IRQ left in service
      blocks its own line and every lower-priority one, and the spurious
      IRQ7/IRQ15 case must *not* be acknowledged at all. Verified with QEMU's
      monitor reporting `EFL=...212` (IF set) with `HLT=1` and `isr=00` on both
      PICs while the kernel idles, and against real unexpected interrupts via
      the `trigger-unhandled-interrupt` / `-unhandled-exception` /
      `-spurious-irq` features, in QEMU and v86 alike.

---

## Phase 2 - Memory management

*Skill: [`.cursor/skills/memory-management/`](.cursor/skills/memory-management/SKILL.md)*

No paging, no heap, no `alloc` yet - only `core`. This phase unblocks
almost everything after it (a real scheduler, filesystem, and shell all
want `Vec`/`Box`/`String`).

- [x] **2.1 - BIOS memory map at boot time.** Have `boot/boot.asm` query
      `INT 15h, EAX=0xE820` for the memory map and stash the resulting list
      at a fixed, pre-agreed address for the kernel to read on startup
      (same spirit as how it already hands off control to `kernel_main`,
      just for memory regions instead of code).
      *Done when:* the kernel can print the discovered memory regions
      (address + length + type) over serial and they look sane for the
      disk image's configured RAM size.
      *Done as:* `detect_memory` in `boot/boot.asm` walks E820 in real mode -
      the only time the BIOS is reachable - and writes a signature, an entry
      count and up to 32 raw 24-byte entries to 0x500, in the low-memory
      scratch area nothing else claims. `kernel/src/memory/map.rs` validates
      that block and reports it: every region over serial, a one-line summary
      on screen. The signature is what lets a BIOS with no E820 support be
      reported as `MEMORY MAP UNAVAILABLE` instead of being mistaken for a
      machine with no RAM. The numbers track the emulator they run on - 127
      MiB usable under QEMU's 128MiB default, 31 MiB under the web demo's
      32MiB v86 - which is the real proof they come from the BIOS.
- [x] **2.2 - Physical frame allocator.** A simple bump or free-list
      allocator over the usable regions from 2.1, operating in 4KiB frames.
      *Done when:* the kernel can allocate and free a handful of frames and
      print their addresses, with no overlaps.
      *Done as:* `kernel/src/memory/frame.rs` - both, in fact: a bump cursor
      walks the usable regions, and frames handed back go on a free list that
      is checked first. The list is *intrusive* (a free frame stores the next
      free frame's index in its own first four bytes), which is how a free
      list exists at all before there is a heap to keep one in. What the BIOS
      calls usable is not the whole story, though: the allocator also carves
      out an explicit list of ranges this kernel has already put something in
      - the IVT/BIOS data area and the E820 handoff block, the boot sector,
      the kernel image (from two new linker symbols, so it tracks the image as
      it grows, `.bss` and both stacks included), and the legacy video/BIOS
      window below 1 MiB. A self-test runs on every boot: eight frames out,
      checked distinct and inside real unreserved RAM, two back, and the next
      allocation must return the more recently freed of them, a double free
      must be refused, and the in-use count must return to zero.
      `scripts/ci-test.sh` re-checks the printed addresses itself rather than
      trusting that verdict - which is what caught the frames in the reserved
      ranges when the reservations were deliberately disabled for one boot.
- [x] **2.3 - Page tables + identity mapping.** Build a page directory and
      page tables (32-bit, non-PAE: directory -> table -> 4KiB page),
      identity-mapping low memory (the simplest correct starting point -
      no higher-half kernel yet).
      *Done when:* paging is enabled (`CR3`/`CR0.PG`) and the kernel keeps
      running and printing normally afterward, in both QEMU and v86.
      *Done as:* `kernel/src/memory/paging.rs` allocates a page directory and
      one page table per 4 MiB from the frame allocator (zeroing each frame
      first - they come back dirty), identity-maps `0..` the top of usable RAM
      rounded up to 4 MiB (so the VGA hole at 0xb8000 is covered too), writes
      that directory into both TSSes via `tss::set_page_directory` (a task
      switch loads `CR3` from the incoming TSS), then loads `CR3` and sets
      `CR0.PG`. The banner reports the mapped window, the table count, the
      `CR3` the CPU accepted, and `PG=1`; surviving the prints after that line
      is the proof the identity map actually covers the kernel.
- [x] **2.4 - Kernel heap + global allocator.** Reserve a heap region,
      implement (or bring in) a simple bump or free-list `#[global_allocator]`.
      *Done when:* `extern crate alloc;` compiles in, and a `Vec<u8>` (or
      similar) can be pushed into and printed without crashing.
      *Done as:* `kernel/src/memory/heap.rs` takes a contiguous 1 MiB run from
      the frame allocator (`allocate_contiguous`, bump-only so the frames are
      one solid physical range), installs a first-fit free-list allocator as
      the crate's `#[global_allocator]`, and brings `alloc` into
      `build-std`. A boot self-test pushes 64 bytes into a `Vec<u8>`, resizes
      it (forcing a second allocation + free), reads the pattern back, and
      checks the used-byte count returns to where it started on drop.
      `scripts/ci-test.sh` greps for that verdict.

- [x] **2.5 - Guard against heap/stack collisions.** Now that both a
      dynamic heap and a fixed-size kernel stack exist, add at least a
      basic sanity check or comment/const documenting their layout so they
      can't silently overlap as both grow.
      *Done when:* the layout (stack range, heap range) is written down
      somewhere in code (not just in your head), and boot still succeeds.
      *Done as:* `kernel/src/memory/layout.rs` documents the address map
      (64 KiB kernel stack and 4 KiB DF stack inside the reserved kernel
      image; 1 MiB heap from free frames outside it), exports
      `KERNEL_STACK_SIZE` / `DOUBLE_FAULT_STACK_SIZE` next to the matching
      `.skip` in `entry.s`, and runs a boot-time check that the live ranges
      are sized correctly, nested correctly, and pairwise disjoint. The
      banner prints the three ranges; `scripts/ci-test.sh` re-parses them
      and re-derives disjointness itself.
- [x] **2.6 - Guard page below the kernel stack.** Now that paging exists,
      leave the page below the kernel stack unmapped so an overflow faults
      immediately instead of silently scribbling over whatever `.bss`
      happens to sit underneath. This is what finishes task 1.4: the
      double-fault handler already has a stack of its own, but a stack
      overflow can't currently be *detected*, so it was verified against a
      different double fault. Place the guard page so it also separates the
      kernel stack from the double-fault handler's private stack (see
      `kernel/src/tss.rs`), which the linker is free to lay out directly
      below it.
      *Done when:* a deliberately-triggered stack overflow (infinite
      recursion behind a `trigger-stack-overflow` feature) prints the
      double-fault report from 1.4 instead of corrupting memory or
      rebooting.
      *Done as:* `entry.s` lays out DF stack | 4 KiB guard | 64 KiB kernel
      stack, page-aligned; `paging::init` identity-maps everything else but
      leaves that guard PTE not-present; `layout::check` asserts the three
      ranges are adjacent and that `paging::is_present` says the guard is
      still unmapped. `KERNEL_FEATURES=trigger-stack-overflow` recurses until
      `esp` grows into the hole; pushing the #PF frame fails and the CPU
      escalates to the vector-8 task gate, printing the double-fault report
      on the private stack.

---

## Phase 3 - Keyboard & timer input

*Skill: [`.cursor/skills/drivers/`](.cursor/skills/drivers/SKILL.md)* (this
phase is really two new drivers, following that skill's conventions)

Needs Phase 1 (interrupts) done first.

- [x] **3.0 - Make printing safe from interrupt context.** `vga::_print` and
      `serial::_print` take a spin lock, so a handler that prints while the
      interrupted code held that lock deadlocks instead of printing - and every
      handler in the kernel prints. Nothing can hit this today (no IRQ line is
      unmasked, so no interrupt can arrive mid-print), but 3.1 is precisely the
      task that unmasks one. Give the writers a reentrancy-safe path (e.g.
      `try_lock` with a fallback, or making handlers print without the lock)
      before a handler starts printing on a real IRQ.
      *Done when:* a handler that fires while a print is in progress reports
      instead of hanging - verified deliberately, e.g. by raising an interrupt
      from inside a print behind a `trigger-*` feature.
      *Done as:* both writers sit behind `sync::IrqMutex`, which `cli`s for the
      critical section (so a maskable IRQ never observes the lock as held) and,
      if something that ignores IF re-enters anyway, still runs the closure
      rather than spinning - the interrupted holder is suspended on this CPU.
      `KERNEL_FEATURES=trigger-print-reentrancy` holds both locks and fires
      `int $0x60`; the catch-all's report lands and the kernel prints
      `print reentrancy ok` afterwards.
- [x] **3.1 - PIT (timer) driver.** Program the 8253/8254 PIT to a fixed
      frequency, add an IRQ0 handler that increments a tick counter.
      *Done when:* the kernel can print an increasing tick count (e.g.
      once a second) over serial, proving interrupts are actually firing.
      *Done as:* `kernel/src/pit.rs` programs channel 0 for 100 Hz (mode 3),
      installs an IRQ0 handler that bumps an `AtomicU32` and EOIs, and
      `pic::unmask(0)` lets the line through. The idle loop wakes on each tick
      and prints `PIT: tick N (S s)` once a second over VGA + serial;
      `scripts/ci-test.sh` checks the banner and that at least two successive
      reports show a strictly increasing counter.
- [x] **3.2 - PS/2 keyboard driver.** Add an IRQ1 handler that reads
      scancodes from the keyboard controller and translates a basic US
      layout (letters, digits, space, enter, backspace) to ASCII.
      *Done when:* typing on the keyboard echoes the corresponding
      characters onto the VGA screen. Test in v86 too - it forwards real
      browser keyboard events into the emulated PS/2 controller, so this
      is a good one to verify "for real" in the web demo, not just QEMU.
      *Done as:* `kernel/src/keyboard.rs` installs an IRQ1 handler that
      reads scancode set 1 from the 8042 data port, tracks Shift, translates
      letters/digits/space/enter/backspace, and echoes to VGA + serial
      (backspace via `vga::backspace`). `pic::unmask(1)` leaves IMR at
      `0xfc/0xff`. Verified with QEMU monitor `sendkey` (typed text appears
      on the VGA screendump and in the serial log) and against the v86 web
      demo.
- [x] **3.3 - Input event queue.** A small fixed-size ring buffer (no heap
      needed, or backed by the new allocator if Phase 2 is done) that
      decouples "a key was pressed" (interrupt context) from "something
      reads and acts on it" (normal code).
      *Done when:* a simple loop in `kernel_main` can drain the queue and
      echo typed characters, with no dropped/duplicated keys under normal
      typing speed.
      *Done as:* `kernel/src/input.rs` is a 64-slot ring buffer behind
      `IrqMutex`. The IRQ1 handler only translates and `push`es; the idle
      loop `pop`s and echoes to VGA + serial. A full queue drops the newest
      event (never blocks the handler) and counts drops. Verified with QEMU
      monitor `sendkey` (typed text appears on the VGA screendump and in the
      serial log with no missing/duplicated characters) and against the v86
      web demo.

---

## Phase 4 - A minimal shell

*Skill: [`.cursor/skills/scheduling-and-processes/`](.cursor/skills/scheduling-and-processes/SKILL.md)
for the scheduler half; [`.cursor/skills/filesystem/`](.cursor/skills/filesystem/SKILL.md)
if/when a command needs disk access.*

Needs Phase 3 (keyboard) for input. Filesystem work (loading/saving files)
is *not* required for a first shell - `help`/`echo`/`clear`-style built-ins
don't need one - so it's tracked as its own optional phase (4.5 below)
rather than blocking this one.

- [x] **4.1 - Line editor.** Build a simple input line buffer on top of the
      Phase 3 keyboard queue: typed characters append to a line, backspace
      removes the last one, enter submits it.
      *Done when:* you can type a line, backspace to fix a typo, and press
      enter to see the whole line echoed back.
      *Done as:* `kernel/src/shell.rs` holds a 72-byte line buffer. The idle
      loop drains the Phase 3 input queue into it: printable characters append
      and echo, backspace pops and erases (VGA + serial `\x08 \x08`), and
      enter submits - printing the whole line back, clearing the buffer, and
      showing a fresh `> ` prompt. Verified with QEMU monitor `sendkey`
      (typo + backspace + enter yields the corrected line on the VGA
      screendump and in the serial log) and against the v86 web demo.
- [x] **4.2 - Built-in commands.** A small command dispatcher with a
      handful of built-ins: `help` (list commands), `clear` (clear the VGA
      screen), `echo <text>`, `about` (prints the GOATos banner/version).
      *Done when:* each built-in works as expected from the prompt, and an
      unrecognized command prints a friendly "unknown command" instead of
      doing nothing or crashing.
      *Done as:* `shell::run_line` splits the submitted buffer on the first
      whitespace word and dispatches `help` / `clear` / `echo` / `about`;
      anything else prints `unknown command: <name>`. Empty lines are a
      no-op (fresh prompt only). `clear` wipes VGA via `vga::clear_screen`
      and marks the serial log with `(screen cleared)`. Verified with QEMU
      monitor `sendkey` for each built-in plus an unknown command (VGA
      screendump + serial), and against the v86 web demo.
- [ ] **4.3 - Cooperative tasks.** Give each "task" (to start: just the
      shell and maybe one background counter/demo task) its own stack and
      a hand-written context switch (save/restore registers including
      `esp`) that runs on an explicit `yield`, not a timer yet.
      *Done when:* two cooperative tasks (e.g. the shell + a task that
      prints a counter) visibly interleave their output.
- [ ] **4.4 - Round-robin scheduler.** A minimal ready-queue that decides
      which task runs next after a yield.
      *Done when:* 3+ tasks round-robin fairly (each gets roughly equal
      turns) over a short run.
- [ ] **4.5 - (optional, needs filesystem) Load & run from disk.** Once a
      basic filesystem exists (see its own skill), extend the shell with a
      command that reads a file's contents (e.g. `cat <file>`) - the first
      real use of on-disk storage beyond booting.
      *Done when:* `cat`-ing a known test file placed on the disk image
      prints its exact contents.

---

## Phase 5 - Graphics mode & a basic GUI

*Skill: [`.cursor/skills/graphics-and-gui/`](.cursor/skills/graphics-and-gui/SKILL.md)*

Explicitly the last phase in this list - don't start it before Phases 1-2
are done, since mouse input (via interrupts) and any nontrivial rendering
work both lean on them.

- [ ] **5.1 - Switch to a graphics mode.** Have `boot.asm` set a VGA (or
      VBE) graphics mode via a real-mode BIOS call before the protected-mode
      switch (the kernel itself has no BIOS access after that point).
      *Done when:* the screen is a solid, known color instead of the text
      console - proving mode-setting worked, in both QEMU and v86.
- [ ] **5.2 - Pixel primitives.** A new `kernel/src/framebuffer.rs` with
      `set_pixel`, `fill_rect`, and `draw_line` over the graphics-mode
      buffer.
      *Done when:* a simple test pattern (a few colored rectangles) draws
      correctly on screen.
- [ ] **5.3 - Bitmap font renderer.** Graphics mode has no built-in text
      font (unlike text mode), so add a small embedded bitmap font and a
      `draw_text` function.
      *Done when:* the existing "GOATos booted successfully" banner
      renders as readable text in graphics mode.
- [ ] **5.4 - PS/2 mouse driver + cursor.** Follows the same driver
      conventions as the Phase 3 keyboard driver; render a simple cursor
      sprite that tracks mouse movement.
      *Done when:* moving the (real or emulated) mouse visibly moves a
      cursor on screen.
- [ ] **5.5 - Minimal windowing.** Basic overlapping rectangular regions
      with a shared framebuffer and simple redraw/damage tracking - enough
      to show two "windows" at once, nothing fancier yet.
      *Done when:* two overlapping rectangles render with correct
      draw order (the "front" one visibly occludes the "back" one).

---

## Notes

- Every task should be verified the way `.cursor/skills/qemu-testing-and-verification/`
  describes: headless serial output for automated/CI-style checks, plus a
  real VGA/framebuffer screenshot for visual confirmation.
- Anything that touches boot-time BIOS calls, disk I/O, or timing-sensitive
  polling should also be spot-checked against the browser demo (see
  `.cursor/skills/web-demo-packaging/`) - several of the trickiest bugs so
  far only showed up under v86, not QEMU.
- This roadmap is a living document - as phases complete, check them off
  here *and* update the [Status list in the README](README.md#status).
