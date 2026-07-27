//! VGA Mode 13h (320×200, 256-color) mode constants and boot paint.
//!
//! The bootloader switches into this mode via BIOS `INT 10h` while still in
//! real mode (`boot/boot.asm`). Once the CPU is in protected mode the BIOS
//! is unreachable, so the kernel never mode-sets itself - it only paints the
//! framebuffer the BIOS left at `0xA0000`.
//!
//! Roadmap 5.1 proved the mode switch with a solid fill. Roadmap 5.2 draws a
//! colored-rectangle test pattern via [`crate::framebuffer`] instead, so a
//! screendump shows more than one DAC index.

use core::fmt;

use crate::framebuffer::{self, BG_COLOR, TEST_RECT_COUNT};

/// BIOS video mode number programmed by the bootloader.
pub const MODE: u8 = 0x13;

/// Horizontal resolution of Mode 13h.
pub const WIDTH: usize = 320;

/// Vertical resolution of Mode 13h.
pub const HEIGHT: usize = 200;

/// Physical address of the Mode 13h linear framebuffer.
pub const FRAMEBUFFER: usize = 0xa0000;

/// One-line description of the graphics mode the bootloader left us in.
#[derive(Clone, Copy)]
pub struct Report {
    /// Background DAC index under the test pattern.
    pub bg_color: u8,
    /// Number of filled rectangles in the test pattern.
    pub rects: usize,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Graphics: VGA mode {:#04x} {}x{} @ {:#010x}, test pattern ({} rects, bg {})",
            MODE, WIDTH, HEIGHT, FRAMEBUFFER, self.rects, self.bg_color
        )
    }
}

/// Paint the Mode 13h framebuffer with the pixel-primitive test pattern and
/// return a banner for the serial (and, until a bitmap font lands, VGA text)
/// log.
pub fn init() -> Report {
    framebuffer::draw_test_pattern();
    Report {
        bg_color: BG_COLOR,
        rects: TEST_RECT_COUNT,
    }
}
