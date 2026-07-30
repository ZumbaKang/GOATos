# GOATos GUI Roadmap

This is the **graphics / GUI** track: framebuffer, fonts, mouse, windowing,
and desktop UI. The core / terminal track lives in
[`ROADMAP-CORE.md`](ROADMAP-CORE.md); the index and track rules are in
[`ROADMAP.md`](ROADMAP.md).

**How to use this file:** an automation (or human) with `TRACK=gui` picks
the **first** unchecked task here (`- [ ]`), implements only that task,
verifies it, checks it off in this file, and opens a PR. Do not start core
shell/FS/scheduler tasks from this file - those belong to the core
automation.

**Ownership (soft):** prefer touching `framebuffer.rs`, `graphics.rs`,
future `gui/` modules, mouse input, and boot video mode-setting. Avoid
drive-by rewrites of `shell` / `fs` / `task` unless a GUI task needs a
tiny shared API (prefer asking for a core-track task via
`Depends on: CORE <task id>`).

**Browser testing:** after deploy, open the GitHub Pages **GUI demo**
(`gui.html`) to see Mode 13h output scaled up, with a live serial log
beside it. Rebuild locally with `./scripts/build-web-demo.sh`.

*Skill: [`.cursor/skills/graphics-and-gui/`](.cursor/skills/graphics-and-gui/SKILL.md)*

---

## Phase 5 - Graphics mode & a basic GUI

*Skill: [`.cursor/skills/graphics-and-gui/`](.cursor/skills/graphics-and-gui/SKILL.md)*

Phases 1-4 (core) are done, so this track can proceed in parallel with
`ROADMAP-CORE.md`. Mouse input and nontrivial rendering still lean on
interrupts + memory from those phases.


- [x] **5.1 - Switch to a graphics mode.** Have `boot.asm` set a VGA (or
      VBE) graphics mode via a real-mode BIOS call before the protected-mode
      switch (the kernel itself has no BIOS access after that point).
      *Done when:* the screen is a solid, known color instead of the text
      console - proving mode-setting worked, in both QEMU and v86.
      *Done as:* `boot/boot.asm` calls `INT 10h / AX=0013h` (VGA mode 13h,
      320x200x256 at `0xA0000`) after A20 and before the protected-mode
      switch. `kernel/src/framebuffer.rs` fills that buffer with palette
      index `0x01` (default DAC dark blue). Serial banner
      `Framebuffer: VGA mode 0x13 320x200x256 at 0xa0000, solid fill 0x01`
      is grepped by CI; QEMU screendump + v86 canvas confirm the solid
      blue screen (text-mode 0xB8000 is no longer displayed).
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

---

## Phase 5 continued - Desktop pieces

After 5.5, keep GUI improvements here so the core track stays free to deepen
shell/FS/scheduling. Add new unchecked tasks below as needed.

- [ ] **5.6 - On-screen terminal window.** Host the existing shell line
      editor inside one GUI window (bitmap font from 5.3), so typing in the
      web GUI demo shows a real prompt on the framebuffer - not only on
      serial / the old text buffer.
      *Done when:* the Pages GUI demo shows a usable `> ` prompt and echo
      on the canvas; serial still mirrors output for CI.
      *Depends on: 5.3, 5.5* (and core shell remaining the command engine).

---

## Notes (GUI track)

- Verify with serial CI checks **and** a real QEMU framebuffer screendump
  (`.cursor/skills/qemu-testing-and-verification/`). Mode-setting and
  painting bugs often look fine in the serial log.
- Always spot-check the browser GUI demo (`web/gui.html` via
  `scripts/build-web-demo.sh` / GitHub Pages) - v86 video paths have
  diverged from QEMU before.
- When finishing the last open GUI milestone listed in README Status,
  check that item off there too.
- If a task needs a core-track deliverable first, mark it
  `Depends on: CORE <task id>` and skip to the next unblocked GUI task.
