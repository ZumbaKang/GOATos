//! VGA Mode 13h linear framebuffer (320x200, 256 colors at 0xA0000).
//!
//! The boot sector switches the BIOS into mode 13h before entering protected
//! mode (see `boot/boot.asm`); this module is the kernel-side write surface
//! for that buffer. It is deliberately separate from [`crate::vga`]'s text-
//! mode driver at 0xB8000 - once graphics mode is active the text buffer is
//! no longer what the monitor shows.
//!
//! Roadmap 5.1 only needs a solid known-color fill to prove the mode switch
//! stuck. Pixel primitives (`set_pixel`, `fill_rect`, `draw_line`) arrive in
//! 5.2; a bitmap font for on-screen text in 5.3.

use core::ptr;

/// BIOS video mode number the bootloader selects (`INT 10h / AH=00h`).
pub const MODE: u8 = 0x13;

/// Physical address of the Mode 13h packed-pixel framebuffer.
pub const BUFFER_ADDR: usize = 0xa0000;

/// Width in pixels.
pub const WIDTH: usize = 320;

/// Height in pixels.
pub const HEIGHT: usize = 200;

/// Bytes in the Mode 13h framebuffer (one byte per pixel).
pub const BUFFER_LEN: usize = WIDTH * HEIGHT;

/// Palette index used for the roadmap 5.1 solid-color proof fill.
///
/// Index `1` is the default VGA DAC entry for dark blue (`#0000AA`). A
/// screendump of a successful boot should be this color edge-to-edge - not
/// the old 80x25 text console.
pub const SOLID_COLOR: u8 = 0x01;

/// Fills the entire Mode 13h framebuffer with `color` (a VGA DAC palette
/// index). Uses volatile stores so the compiler cannot coalesce or drop the
/// writes to memory-mapped VGA.
pub fn fill(color: u8) {
    let buffer = BUFFER_ADDR as *mut u8;
    for i in 0..BUFFER_LEN {
        // SAFETY: `BUFFER_ADDR` is the Mode 13h framebuffer the bootloader
        // put the VGA hardware into; the identity map (and pre-paging
        // physical addressing) covers the legacy hole below 1 MiB, so each
        // byte in `0..BUFFER_LEN` is a writable pixel. Nothing else owns
        // this range.
        unsafe { ptr::write_volatile(buffer.add(i), color) };
    }
}
