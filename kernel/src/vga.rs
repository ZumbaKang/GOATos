//! Minimal VGA text-mode (0xB8000) driver.
//!
//! This is GOATos's "displayable" output surface: real BIOS/QEMU render this
//! buffer as text on a monitor, and browser-based x86 emulators (e.g. v86)
//! render the exact same buffer to a `<canvas>` - which is what lets GOATos
//! be shown on a web page with no extra work on our end.

use core::fmt;
use core::ptr;

use crate::sync::IrqMutex;

const BUFFER_ADDR: usize = 0xb8000;
const BUFFER_WIDTH: usize = 80;
const BUFFER_HEIGHT: usize = 25;

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Color {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    Pink = 13,
    Yellow = 14,
    White = 15,
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
struct ColorCode(u8);

impl ColorCode {
    const fn new(foreground: Color, background: Color) -> ColorCode {
        ColorCode(((background as u8) << 4) | (foreground as u8))
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
struct ScreenChar {
    ascii_character: u8,
    color_code: ColorCode,
}

struct Writer {
    column: usize,
    row: usize,
    color_code: ColorCode,
}

impl Writer {
    fn buffer() -> *mut ScreenChar {
        BUFFER_ADDR as *mut ScreenChar
    }

    fn put_char(&self, row: usize, col: usize, ch: ScreenChar) {
        let index = row * BUFFER_WIDTH + col;
        unsafe { ptr::write_volatile(Self::buffer().add(index), ch) };
    }

    fn get_char(&self, row: usize, col: usize) -> ScreenChar {
        let index = row * BUFFER_WIDTH + col;
        unsafe { ptr::read_volatile(Self::buffer().add(index)) }
    }

    fn write_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.new_line(),
            byte => {
                if self.column >= BUFFER_WIDTH {
                    self.new_line();
                }
                self.put_char(
                    self.row,
                    self.column,
                    ScreenChar {
                        ascii_character: byte,
                        color_code: self.color_code,
                    },
                );
                self.column += 1;
            }
        }
    }

    fn new_line(&mut self) {
        if self.row + 1 < BUFFER_HEIGHT {
            self.row += 1;
        } else {
            // Scroll everything up one row and clear the freed bottom row.
            for row in 1..BUFFER_HEIGHT {
                for col in 0..BUFFER_WIDTH {
                    let ch = self.get_char(row, col);
                    self.put_char(row - 1, col, ch);
                }
            }
            self.clear_row(BUFFER_HEIGHT - 1);
        }
        self.column = 0;
    }

    fn clear_row(&self, row: usize) {
        let blank = ScreenChar {
            ascii_character: b' ',
            color_code: self.color_code,
        };
        for col in 0..BUFFER_WIDTH {
            self.put_char(row, col, blank);
        }
    }

    fn write_string(&mut self, s: &str) {
        for byte in s.bytes() {
            match byte {
                // printable ASCII or newline
                0x20..=0x7e | b'\n' => self.write_byte(byte),
                // everything else is unrepresentable in code page 437 text mode
                _ => self.write_byte(0xfe),
            }
        }
    }
}

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_string(s);
        Ok(())
    }
}

/// Protected by [`IrqMutex`] so an interrupt handler that prints while the
/// interrupted code was mid-print reports instead of deadlocking on a spin
/// lock (roadmap 3.0).
static WRITER: IrqMutex<Writer> = IrqMutex::new(Writer {
    column: 0,
    row: 0,
    color_code: ColorCode::new(Color::White, Color::Black),
});

/// Clears the whole screen and resets the cursor to the top-left corner.
pub fn clear_screen() {
    WRITER.with(|writer| {
        for row in 0..BUFFER_HEIGHT {
            writer.clear_row(row);
        }
        writer.row = 0;
        writer.column = 0;
    });
}

/// Erases the character left of the cursor and moves the cursor back one
/// column. No-op at column 0 (does not wrap onto the previous row) - enough
/// for the keyboard driver's basic echo; a real line editor can do better.
pub fn backspace() {
    WRITER.with(|writer| {
        if writer.column == 0 {
            return;
        }
        writer.column -= 1;
        writer.put_char(
            writer.row,
            writer.column,
            ScreenChar {
                ascii_character: b' ',
                color_code: writer.color_code,
            },
        );
    });
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    WRITER.with(|writer| {
        writer
            .write_fmt(args)
            .expect("writing to VGA buffer failed");
    });
}

/// Runs `f` while the VGA writer lock is held.
///
/// Used by the print-reentrancy self-test to raise an interrupt from inside a
/// critical section; ordinary callers should use [`vga_print!`] /
/// [`vga_println!`] instead.
#[doc(hidden)]
pub fn with_lock_held<R>(f: impl FnOnce() -> R) -> R {
    WRITER.with(|_| f())
}

/// Prints to the VGA text-mode screen, without a trailing newline.
#[macro_export]
macro_rules! vga_print {
    ($($arg:tt)*) => {
        $crate::vga::_print(format_args!($($arg)*));
    };
}

/// Prints to the VGA text-mode screen, appending a newline.
#[macro_export]
macro_rules! vga_println {
    () => ($crate::vga_print!("\n"));
    ($fmt:expr) => ($crate::vga_print!(concat!($fmt, "\n")));
    ($fmt:expr, $($arg:tt)*) => ($crate::vga_print!(concat!($fmt, "\n"), $($arg)*));
}
