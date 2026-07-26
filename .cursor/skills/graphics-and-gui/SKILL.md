---
name: graphics-and-gui
description: Guidance for eventually building a graphical UI for GOATos (a pixel-graphics framebuffer, mouse support, and basic windowing) - explicitly a later-stage goal, not a first step. Use this when the user asks for GUI/graphics work, or when starting on a graphics mode framebuffer driver.
---

# Graphics and GUI (future work, not a first step)

GOATos currently only has VGA **text mode** (`kernel/src/vga.rs`, the
80x25 character buffer at `0xB8000`) - which is exactly what makes it
"displayable" in both real BIOS/QEMU and the browser-based v86 demo (see
`web-demo-packaging`) with almost no code. A graphical UI is a deliberately
later milestone built on top of that same "just write to a buffer that the
BIOS/emulator already renders for you" idea, one level up.

## Suggested order of implementation

1. **Switch to a VGA graphics mode** (or VESA/VBE for higher resolutions
   and more colors) instead of text mode, via BIOS video mode-setting calls
   made from `boot/boot.asm` before the protected-mode switch (BIOS video
   services are real-mode/BIOS-call-based, so this has to happen while
   still in real mode - the kernel itself has no way to call the BIOS
   after the jump to protected mode, matching how `entry.s`/`kernel_main`
   currently have no BIOS access either).
2. **A pixel/framebuffer drawing module** (`kernel/src/framebuffer.rs` or
   similar): basic primitives first - set-pixel, fill-rect, blit - then a
   simple bitmap font renderer for text, since VGA text mode's built-in
   font goes away once you're in a graphics mode.
3. **Mouse input**, via the PS/2 mouse (needs interrupts - see
   `interrupts-and-exceptions` - and follows the same driver conventions as
   `drivers`).
4. **Minimal windowing**, only once the above is solid: even just
   "multiple overlapping rectangular regions with a shared framebuffer and
   basic redraw/damage tracking" is a reasonable first version. Don't
   over-build a full window manager before there's anything (a shell,
   basic apps) to actually put in windows.

## Conventions to follow

- Keep this cleanly separated from `vga.rs`'s text-mode code; a graphics
  framebuffer driver is a different enough abstraction that it shouldn't
  try to share an implementation with text mode, even though both target
  the same physical hardware.
- Whatever mode is chosen must still work under v86 in the browser demo -
  test any new video mode there too, not just in QEMU (see
  `web-demo-packaging`, especially the "boots in QEMU but not in v86"
  debugging process, since video mode support is a plausible place for the
  two to diverge).
- This is explicitly a "later" milestone per the project's own roadmap
  (see the repo README) - don't start it before more foundational pieces
  (`memory-management`, `interrupts-and-exceptions`) are in place, since
  graphics/mouse work leans on both.
