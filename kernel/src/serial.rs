//! Minimal COM1 (0x3F8) serial port driver used to observe kernel output
//! from a headless QEMU instance (`-serial stdio`), since there is no
//! framebuffer/display support yet.

use core::fmt::Write;
use spin::Mutex;
use uart_16550::backend::PioBackend;
use uart_16550::{Config, Uart16550Tty};

const COM1_PORT: u16 = 0x3f8;

static SERIAL1: Mutex<Option<Uart16550Tty<PioBackend>>> = Mutex::new(None);

/// Initializes the COM1 serial port. Must be called once before any of the
/// `serial_print!`/`serial_println!` macros are used.
///
/// # Safety
/// Must only be called once, and only on a machine where COM1 is either a
/// real or emulated 16550-compatible UART (true for BIOS/UEFI PCs and QEMU).
pub fn init() {
    let uart = unsafe {
        Uart16550Tty::new_port(COM1_PORT, Config::default())
            .expect("failed to initialize COM1 serial port")
    };
    *SERIAL1.lock() = Some(uart);
}

#[doc(hidden)]
pub fn _print(args: core::fmt::Arguments) {
    SERIAL1
        .lock()
        .as_mut()
        .expect("serial port used before serial::init() was called")
        .write_fmt(args)
        .expect("writing to serial port failed");
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
