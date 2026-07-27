//! Mode 13h pixel primitives over the VGA framebuffer.
//!
//! Builds on [`crate::graphics`]: the bootloader has already switched into
//! Mode 13h and left a linear 320×200 buffer at `0xA0000`. This module only
//! paints into that buffer - it does not mode-set, touch the DAC palette, or
//! share any code with the text-mode driver in [`crate::vga`].
//!
//! Roadmap 5.2: `set_pixel`, `fill_rect`, `draw_line`, plus a boot test
//! pattern of colored rectangles so a screendump proves the primitives work.

use core::ptr;

use crate::graphics::{FRAMEBUFFER, HEIGHT, WIDTH};

/// Background DAC index for the boot test pattern (black).
pub const BG_COLOR: u8 = 0;

/// Number of filled rectangles in [`draw_test_pattern`].
pub const TEST_RECT_COUNT: usize = 4;

/// One rectangle in the boot test pattern (`x`, `y`, `w`, `h`, DAC color).
const TEST_RECTS: [(i32, i32, i32, i32, u8); TEST_RECT_COUNT] = [
    (20, 20, 80, 50, 4),   // red
    (120, 30, 70, 60, 2),  // green
    (210, 20, 90, 40, 1),  // blue
    (40, 100, 240, 70, 14), // yellow
];

/// Endpoints of the diagonal line drawn across the test pattern.
const TEST_LINE: (i32, i32, i32, i32, u8) = (10, 180, 310, 10, 15); // white

/// Writes a single pixel at `(x, y)` with VGA DAC index `color`.
///
/// Out-of-bounds coordinates are ignored (no panic) so callers can clip
/// naively without taking the machine down.
pub fn set_pixel(x: i32, y: i32, color: u8) {
    if x < 0 || y < 0 || x >= WIDTH as i32 || y >= HEIGHT as i32 {
        return;
    }
    let offset = (y as usize) * WIDTH + (x as usize);
    let fb = FRAMEBUFFER as *mut u8;
    // SAFETY: `offset` is in `0..WIDTH*HEIGHT` after the bounds check above,
    // and that range is exactly the Mode 13h framebuffer the bootloader left
    // at `FRAMEBUFFER`.
    unsafe { ptr::write_volatile(fb.add(offset), color) };
}

/// Fills the axis-aligned rectangle with top-left `(x, y)` and size
/// `(w, h)` using VGA DAC index `color`.
///
/// Negative sizes are treated as empty. The rectangle is clipped to the
/// framebuffer so partial on-screen regions still paint.
pub fn fill_rect(x: i32, y: i32, w: i32, h: i32, color: u8) {
    if w <= 0 || h <= 0 {
        return;
    }
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x.saturating_add(w)).min(WIDTH as i32);
    let y1 = (y.saturating_add(h)).min(HEIGHT as i32);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    for py in y0..y1 {
        for px in x0..x1 {
            set_pixel(px, py, color);
        }
    }
}

/// Draws a 1-pixel-wide line from `(x0, y0)` to `(x1, y1)` with VGA DAC
/// index `color`, using Bresenham's algorithm.
///
/// Endpoints (and intermediate samples) outside the framebuffer are skipped
/// via [`set_pixel`]'s bounds check.
pub fn draw_line(mut x0: i32, mut y0: i32, x1: i32, y1: i32, color: u8) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        set_pixel(x0, y0, color);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

/// Clears the framebuffer and paints the roadmap 5.2 test pattern: a black
/// background, a handful of colored rectangles, and one diagonal line that
/// exercises [`draw_line`].
pub fn draw_test_pattern() {
    fill_rect(0, 0, WIDTH as i32, HEIGHT as i32, BG_COLOR);
    for &(x, y, w, h, color) in &TEST_RECTS {
        fill_rect(x, y, w, h, color);
    }
    let (x0, y0, x1, y1, color) = TEST_LINE;
    draw_line(x0, y0, x1, y1, color);
}
