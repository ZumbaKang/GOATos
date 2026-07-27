//! 8253/8254 Programmable Interval Timer - channel 0 on IRQ0.
//!
//! Channel 0's output is wired to the master PIC's IRQ0 line, so programming
//! it to a fixed frequency and unmasking that line is how the kernel learns
//! that time is passing. This module:
//!
//! 1. installs an IRQ0 handler that increments a tick counter and
//!    acknowledges the PIC,
//! 2. programs channel 0 for [`FREQUENCY_HZ`] (mode 3 square wave),
//! 3. unmasks IRQ0 so the interrupts actually arrive.
//!
//! Nothing here prints from the handler itself - a flood of IRQ0 diagnostics
//! would bury every other message - so the boot path polls [`ticks`] from the
//! idle loop and reports once a second over serial.

use core::arch::asm;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::idt::{self, StackFrame};
use crate::pic::{self, IRQ_VECTOR_BASE};

/// IRQ line the PIT's channel 0 drives.
pub const IRQ: u8 = 0;
/// Vector that line raises after [`crate::pic::init`]'s remap.
pub const VECTOR: u8 = IRQ_VECTOR_BASE + IRQ;

/// How often channel 0 raises IRQ0. 100 Hz is a common kernel tick rate:
/// short enough that a one-second report is exact (`ticks % 100 == 0`), long
/// enough that the handler is not the busiest thing in the machine.
pub const FREQUENCY_HZ: u32 = 100;

/// Oscillator feeding the three PIT channels, in Hz. Not a round number - it
/// is 105/88 MHz, the historical ISA bus clock divided by 3.
const PIT_OSCILLATOR_HZ: u32 = 1_193_182;

/// Divisor loaded into channel 0 so it fires at [`FREQUENCY_HZ`].
pub const DIVISOR: u16 = (PIT_OSCILLATOR_HZ / FREQUENCY_HZ) as u16;

/// Channel 0 data port (write the divisor low byte, then high byte).
const CHANNEL0_DATA: u16 = 0x40;
/// Mode/command register shared by all three channels.
const COMMAND: u16 = 0x43;

/// Command byte: channel 0, lobyte/hibyte access, mode 3 (square wave),
/// binary counting. `0b00_11_011_0`.
const CMD_CHANNEL0_MODE3: u8 = 0x36;

/// Ticks since [`init`]. Wrapping is fine - at 100 Hz a `u32` lasts ~497 days,
/// and consumers that care about absolute time are nowhere near ready.
static TICKS: AtomicU32 = AtomicU32::new(0);

/// Programs channel 0, installs the IRQ0 handler, and unmasks the line.
///
/// Must run after [`crate::interrupts::init`] (so there is a catch-all to
/// replace on [`VECTOR`]) and preferably after [`crate::interrupts::enable`]
/// has already reported the all-masked state - unmasking here is what first
/// lets a hardware IRQ through.
///
/// Infallible like the other drivers: the PIT has no status to fail on, and a
/// kernel that cannot tick still boots; it just never sees the counter move.
pub fn init() {
    TICKS.store(0, Ordering::Relaxed);

    // SAFETY: IRQ0 pushes no error code, and `VECTOR` is exactly the one the
    // PIC remap pointed that line at. Replaces the catch-all `interrupts::init`
    // installed for the same vector.
    unsafe {
        idt::set_handler(VECTOR, timer_interrupt);
    }

    // SAFETY: these ports belong to the PIT, and writing the command byte
    // then the two divisor bytes is the documented programming sequence. No
    // other code talks to the PIT yet.
    unsafe {
        outb(COMMAND, CMD_CHANNEL0_MODE3);
        outb(CHANNEL0_DATA, (DIVISOR & 0xff) as u8);
        outb(CHANNEL0_DATA, (DIVISOR >> 8) as u8);
    }

    // Handler is registered and the chip is counting - safe to let IRQ0 in.
    pic::unmask(IRQ);
}

/// Ticks observed since [`init`]. Read from ordinary code; the handler is the
/// only writer.
pub fn ticks() -> u32 {
    TICKS.load(Ordering::Relaxed)
}

/// Whole seconds implied by [`ticks`] at [`FREQUENCY_HZ`].
pub fn seconds() -> u32 {
    ticks() / FREQUENCY_HZ
}

extern "x86-interrupt" fn timer_interrupt(_frame: StackFrame) {
    TICKS.fetch_add(1, Ordering::Relaxed);
    // Acknowledge before anything else could re-enable interrupts: leaving
    // IRQ0 in service blocks it and every lower-priority line.
    let _ = pic::end_of_interrupt(IRQ);
}

/// Writes `value` to I/O port `port`.
///
/// # Safety
/// As for [`crate::pic`]: the caller must know `port` is the device it thinks
/// and that `value` is meaningful there.
unsafe fn outb(port: u16, value: u8) {
    unsafe {
        asm!(
            "outb %al, %dx",
            in("dx") port,
            in("al") value,
            options(att_syntax, nomem, nostack, preserves_flags),
        );
    }
}
