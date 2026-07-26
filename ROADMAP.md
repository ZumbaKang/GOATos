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

Right now the kernel runs with interrupts off and no IDT of its own. Any
bug that trips a CPU exception has undefined, unrecoverable behavior (most
likely a silent reboot). This phase turns "the kernel just froze" into a
readable diagnostic, which makes every phase after it much faster to debug.

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
- [ ] **1.4 - Double-fault handler with its own stack.** Set up a TSS with
      a dedicated stack for the double-fault handler (vector 8). Without
      this, a fault *while handling a fault* (e.g. a stack overflow)
      triple-faults the CPU and silently reboots the VM instead of printing
      anything.
      *Done when:* a deliberately-triggered stack overflow prints a
      double-fault message instead of rebooting.
- [ ] **1.5 - Remap the PIC.** Reprogram the 8259 PIC so hardware IRQs land
      on vectors 32+ instead of colliding with the CPU exception vectors
      (0-31) - a classic, well-documented gotcha.
      *Done when:* the PIC is remapped and interrupts are still masked (no
      handlers yet, so nothing should fire).
- [ ] **1.6 - Enable interrupts.** Re-enable interrupts (`sti`) now that
      there's an IDT and PIC in a known state. Add a "spurious/unhandled
      interrupt" default handler for any vector without a real one yet, so
      an unexpected IRQ prints something instead of crashing.
      *Done when:* the kernel boots normally with interrupts enabled and
      idles (via `hlt`) without any unexpected crashes.

---

## Phase 2 - Memory management

*Skill: [`.cursor/skills/memory-management/`](.cursor/skills/memory-management/SKILL.md)*

No paging, no heap, no `alloc` yet - only `core`. This phase unblocks
almost everything after it (a real scheduler, filesystem, and shell all
want `Vec`/`Box`/`String`).

- [ ] **2.1 - BIOS memory map at boot time.** Have `boot/boot.asm` query
      `INT 15h, EAX=0xE820` for the memory map and stash the resulting list
      at a fixed, pre-agreed address for the kernel to read on startup
      (same spirit as how it already hands off control to `kernel_main`,
      just for memory regions instead of code).
      *Done when:* the kernel can print the discovered memory regions
      (address + length + type) over serial and they look sane for the
      disk image's configured RAM size.
- [ ] **2.2 - Physical frame allocator.** A simple bump or free-list
      allocator over the usable regions from 2.1, operating in 4KiB frames.
      *Done when:* the kernel can allocate and free a handful of frames and
      print their addresses, with no overlaps.
- [ ] **2.3 - Page tables + identity mapping.** Build a page directory and
      page tables (32-bit, non-PAE: directory -> table -> 4KiB page),
      identity-mapping low memory (the simplest correct starting point -
      no higher-half kernel yet).
      *Done when:* paging is enabled (`CR3`/`CR0.PG`) and the kernel keeps
      running and printing normally afterward, in both QEMU and v86.
- [ ] **2.4 - Kernel heap + global allocator.** Reserve a heap region,
      implement (or bring in) a simple bump or free-list `#[global_allocator]`.
      *Done when:* `extern crate alloc;` compiles in, and a `Vec<u8>` (or
      similar) can be pushed into and printed without crashing.
- [ ] **2.5 - Guard against heap/stack collisions.** Now that both a
      dynamic heap and a fixed-size kernel stack exist, add at least a
      basic sanity check or comment/const documenting their layout so they
      can't silently overlap as both grow.
      *Done when:* the layout (stack range, heap range) is written down
      somewhere in code (not just in your head), and boot still succeeds.

---

## Phase 3 - Keyboard & timer input

*Skill: [`.cursor/skills/drivers/`](.cursor/skills/drivers/SKILL.md)* (this
phase is really two new drivers, following that skill's conventions)

Needs Phase 1 (interrupts) done first.

- [ ] **3.1 - PIT (timer) driver.** Program the 8253/8254 PIT to a fixed
      frequency, add an IRQ0 handler that increments a tick counter.
      *Done when:* the kernel can print an increasing tick count (e.g.
      once a second) over serial, proving interrupts are actually firing.
- [ ] **3.2 - PS/2 keyboard driver.** Add an IRQ1 handler that reads
      scancodes from the keyboard controller and translates a basic US
      layout (letters, digits, space, enter, backspace) to ASCII.
      *Done when:* typing on the keyboard echoes the corresponding
      characters onto the VGA screen. Test in v86 too - it forwards real
      browser keyboard events into the emulated PS/2 controller, so this
      is a good one to verify "for real" in the web demo, not just QEMU.
- [ ] **3.3 - Input event queue.** A small fixed-size ring buffer (no heap
      needed, or backed by the new allocator if Phase 2 is done) that
      decouples "a key was pressed" (interrupt context) from "something
      reads and acts on it" (normal code).
      *Done when:* a simple loop in `kernel_main` can drain the queue and
      echo typed characters, with no dropped/duplicated keys under normal
      typing speed.

---

## Phase 4 - A minimal shell

*Skill: [`.cursor/skills/scheduling-and-processes/`](.cursor/skills/scheduling-and-processes/SKILL.md)
for the scheduler half; [`.cursor/skills/filesystem/`](.cursor/skills/filesystem/SKILL.md)
if/when a command needs disk access.*

Needs Phase 3 (keyboard) for input. Filesystem work (loading/saving files)
is *not* required for a first shell - `help`/`echo`/`clear`-style built-ins
don't need one - so it's tracked as its own optional phase (4.5 below)
rather than blocking this one.

- [ ] **4.1 - Line editor.** Build a simple input line buffer on top of the
      Phase 3 keyboard queue: typed characters append to a line, backspace
      removes the last one, enter submits it.
      *Done when:* you can type a line, backspace to fix a typo, and press
      enter to see the whole line echoed back.
- [ ] **4.2 - Built-in commands.** A small command dispatcher with a
      handful of built-ins: `help` (list commands), `clear` (clear the VGA
      screen), `echo <text>`, `about` (prints the GOATos banner/version).
      *Done when:* each built-in works as expected from the prompt, and an
      unrecognized command prints a friendly "unknown command" instead of
      doing nothing or crashing.
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
