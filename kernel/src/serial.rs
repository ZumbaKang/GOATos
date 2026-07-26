//! Minimal COM1 (0x3F8) serial port driver used to observe kernel output
//! from a headless QEMU instance (`-serial stdio`) or from CI, since
//! automated tests can't "look at" the VGA screen the way a human (or v86 in
//! a browser) can.
//!
//! Deliberately skips `Uart16550::test_loopback()`/`check_connected()`: they
//! aren't necessary just to *send* bytes, and loopback mode isn't reliably
//! emulated by every x86 emulator (notably: it hangs under v86).

use core::fmt::Write;
use spin::Mutex;
use uart_16550::backend::PioBackend;
use uart_16550::{Config, Uart16550};

const COM1_PORT: u16 = 0x3f8;

static SERIAL1: Mutex<Option<Uart16550<PioBackend>>> = Mutex::new(None);

/// Initializes the COM1 serial port. Safe to call even if no serial port is
/// actually present/emulated: if setup fails, `serial_print!`/`serial_println!`
/// simply become no-ops rather than panicking. Serial output is a "best
/// effort" debugging aid (mainly for headless QEMU/CI); it must never be
/// able to wedge the kernel, since the VGA display is the primary,
/// load-bearing output surface.
///
/// # Safety
/// Must only be called once, and only on a machine where COM1 - if present -
/// is a real or emulated 16550-compatible UART (true for BIOS PCs and QEMU).
pub fn init() {
    let Ok(mut uart) = (unsafe { Uart16550::new_port(COM1_PORT) }) else {
        return;
    };
    if uart.init(Config::default()).is_err() {
        return;
    }
    *SERIAL1.lock() = Some(uart);
}

struct SerialWriter;

impl Write for SerialWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        if let Some(uart) = SERIAL1.lock().as_mut() {
            uart.send_bytes_exact(s.as_bytes());
        }
        Ok(())
    }
}

#[doc(hidden)]
pub fn _print(args: core::fmt::Arguments) {
    let _ = SerialWriter.write_fmt(args);
}

/// Prints to the host through the serial interface, without a trailing newline.
#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {
        $crate::serial::_print(format_args!($($arg)*));
    };
}

/// Prints to the host through the serial interface, appending a newline.
#[macro_export]
macro_rules! serial_println {
    () => ($crate::serial_print!("\n"));
    ($fmt:expr) => ($crate::serial_print!(concat!($fmt, "\n")));
    ($fmt:expr, $($arg:tt)*) => ($crate::serial_print!(concat!($fmt, "\n"), $($arg)*));
}
