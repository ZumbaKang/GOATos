---
name: graphics-and-gui
description: Guidance for GOATos graphics (Mode 13h is on; pixel primitives, bitmap font, mouse, and windowing are next). Use this when working on framebuffer/GUI code or changing the boot-time video mode.
---

# Graphics and GUI

GOATos boots into VGA **Mode 13h** (320x200x256, linear framebuffer at
`0xA0000`): `boot/boot.asm` calls `INT 10h / AX=0013h` in real mode, and
`kernel/src/framebuffer.rs` owns the on-screen pixels. The old text-mode
driver (`kernel/src/vga.rs` at `0xB8000`) still exists for bring-up banners
until a bitmap font lands, but it is not what QEMU/v86 display. Same idea
as before - write to a buffer the BIOS/emulator already renders - one level
up from characters to pixels.

## What is already in place (roadmap 5.1)

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
  two to diverge). Mode 13h is the conservative choice for that reason.
  On GitHub Pages / `_site/`, open **`gui.html`** (scaled canvas + serial
  log) to verify graphics — not only the hub page.
- GUI tasks live in [`ROADMAP-GUI.md`](../../../ROADMAP-GUI.md) and are
  picked by automations with `TRACK=gui`. Core/shell/FS work stays on
  `ROADMAP-CORE.md` so the tracks can advance in parallel.
- Do not grow `boot/boot.asm` casually - it is still on a 512-byte budget
  (see `bootloader-and-linking`). Higher modes (VBE) likely need a
  second-stage loader or a real-mode helper stub, not more MBR bytes.
