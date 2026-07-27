//! VGA Mode 13h (320×200, 256-color) solid fill.
//!
//! The bootloader switches into this mode via BIOS `INT 10h` while still in
//! real mode (`boot/boot.asm`). Once the CPU is in protected mode the BIOS
//! is unreachable, so the kernel never mode-sets itself - it only paints the
//! framebuffer the BIOS left at `0xA0000`.
//!
//! This module is deliberately tiny: roadmap 5.1 only needs a solid known
//! color proving the mode switch worked. Pixel primitives live in a later
//! `framebuffer` module (roadmap 5.2).

use core::fmt;
use core::ptr;

/// BIOS video mode number programmed by the bootloader.
pub const MODE: u8 = 0x13;

/// Horizontal resolution of Mode 13h.
pub const WIDTH: usize = 320;

/// Vertical resolution of Mode 13h.
pub const HEIGHT: usize = 200;

/// Physical address of the Mode 13h linear framebuffer.
pub const FRAMEBUFFER: usize = 0xa0000;

/// Default VGA DAC index used for the boot solid fill (palette blue).
/// Distinct from black so a failed mode-set (still text mode / blank) is
/// obvious on a screendump.
pub const FILL_COLOR: u8 = 1;

/// Bytes in the Mode 13h framebuffer (`WIDTH * HEIGHT`).
const FRAMEBUFFER_LEN: usize = WIDTH * HEIGHT;

/// Fills the entire Mode 13h framebuffer with `color` (a VGA DAC index).
///
/// Safe to call before or after paging: the bootloader sets Mode 13h before
/// the protected-mode jump, and once paging is on the identity map covers
/// the legacy video hole below 1 MiB (including `0xA0000`).
pub fn fill_solid(color: u8) {
    let fb = FRAMEBUFFER as *mut u8;
    // Volatile: the framebuffer is a memory-mapped device the VGA hardware
    // (and emulators) sample independently of the CPU cache.
    for i in 0..FRAMEBUFFER_LEN {
        // SAFETY: `fb` points at the Mode 13h framebuffer the bootloader
        // left mapped at `FRAMEBUFFER`; every offset below `FRAMEBUFFER_LEN`
        // is inside that buffer.
        unsafe { ptr::write_volatile(fb.add(i), color) };
    }
}

/// One-line description of the graphics mode the bootloader left us in.
#[derive(Clone, Copy)]
pub struct Report {
    /// VGA DAC index written across the whole framebuffer.
    pub fill_color: u8,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Graphics: VGA mode {:#04x} {}x{} @ {:#010x}, fill color {} (solid)",
            MODE, WIDTH, HEIGHT, FRAMEBUFFER, self.fill_color
        )
    }
}

/// Paint the Mode 13h framebuffer solid and return a banner for the serial
/// (and, until text mode is gone for good, VGA text) log.
pub fn init() -> Report {
    fill_solid(FILL_COLOR);
    Report {
        fill_color: FILL_COLOR,
    }
}
