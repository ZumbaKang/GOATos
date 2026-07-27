---
name: graphics-and-gui
description: Guidance for GOATos graphics (Mode 13h + pixel primitives are on; bitmap font, mouse, and windowing are next). Use this when working on framebuffer/GUI code or changing the boot-time video mode.
---

# Graphics and GUI

GOATos boots into VGA **Mode 13h** (320×200, 256-color, framebuffer at
`0xA0000`): `boot/boot.asm` sets it via BIOS `INT 10h` before the
protected-mode switch, and `kernel/src/graphics.rs` paints a colored
rectangle test pattern via `kernel/src/framebuffer.rs` as the first thing
`kernel_main` does (roadmap 5.1 / 5.2). The old text-mode driver
(`kernel/src/vga.rs`, `0xB8000`) still runs for serial-mirrored banners and
the shell, but the hardware (and v86 canvas) no longer shows it - that is
intentional until a bitmap font (5.3) restores on-screen text.

## What is already in place (roadmap 5.1 / 5.2)

- Mode set in `boot/boot.asm` (`mov ax, 0x0013` / `int 0x10`) - five bytes;
  the GDT `align 8` slack absorbs them so the 512-byte sector still fits.
- `kernel/src/graphics.rs` - mode constants + boot init that draws the
  test pattern and reports mode / resolution / framebuffer address.
- `kernel/src/framebuffer.rs` - `set_pixel`, `fill_rect`, `draw_line`
  (Bresenham), plus `draw_test_pattern` (black bg, four colored rects,
  white diagonal).

## Suggested order from here

1. ~~**Switch to a VGA graphics mode.**~~ Done (5.1): Mode 13h + solid fill.
2. ~~**A pixel/framebuffer drawing module.**~~ Done (5.2): primitives +
   rectangle test pattern. Next: a simple bitmap font renderer for text,
   since VGA text mode's built-in font goes away once you're in a graphics
   mode.
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
  two to diverge). Mode 13h is the conservative choice for that reason.
- Do not grow `boot/boot.asm` casually - it is still on a 512-byte budget
  (see `bootloader-and-linking`). Higher modes (VBE) likely need a
  second-stage loader or a real-mode helper stub, not more MBR bytes.
