---
name: graphics-and-gui
description: Guidance for eventually building a graphical UI for GOATos (a pixel-graphics framebuffer, mouse support, and basic windowing) - explicitly a later-stage goal, not a first step. Use this when the user asks for GUI/graphics work, or when starting on a graphics mode framebuffer driver.
---

# Graphics and GUI (future work, not a first step)

GOATos boots into VGA **Mode 13h** (320x200x256, linear framebuffer at
`0xA0000`): `boot/boot.asm` calls `INT 10h / AX=0013h` in real mode, and
`kernel/src/framebuffer.rs` owns the on-screen pixels. The old text-mode
driver (`kernel/src/vga.rs` at `0xB8000`) still exists for bring-up banners
until a bitmap font lands, but it is not what QEMU/v86 display. Same idea
as before - write to a buffer the BIOS/emulator already renders - one level
up from characters to pixels.

## Suggested order of implementation

1. **Switch to a VGA graphics mode** - **done (roadmap 5.1)**: Mode 13h via
   BIOS in `boot/boot.asm`, solid-color fill in `framebuffer.rs`. Stick
   with Mode 13h for later tasks unless there is a concrete reason to move
   to VBE (higher res / more colors); Mode 13h is what was verified under
   both QEMU and v86.
2. **Pixel/framebuffer drawing primitives** on the existing
   `kernel/src/framebuffer.rs`: `set_pixel`, `fill_rect`, `draw_line` (and
   later blit), then a simple bitmap font renderer for text, since VGA text
   mode's built-in font is gone in graphics mode.
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
