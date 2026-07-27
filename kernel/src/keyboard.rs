//! PS/2 keyboard driver - IRQ1, scancode set 1, basic US QWERTY.
//!
//! The 8042 keyboard controller raises IRQ1 whenever a byte is waiting in its
//! output buffer (port 0x60). This module:
//!
//! 1. installs an IRQ1 handler that reads that byte,
//! 2. tracks Shift so letters can come out upper- or lower-case,
//! 3. translates a basic US layout (letters, digits, space, enter, backspace)
//!    to a [`crate::input::KeyEvent`] and pushes it onto the input queue,
//! 4. unmasks IRQ1 so the interrupts actually arrive.
//!
//! Echoing / acting on keys is the consumer's job (see roadmap 3.3) - the
//! handler only enqueues. BIOS/firmware already leaves the controller in
//! scancode set 1 on every machine this kernel targets, so there is no
//! controller programming step. Extended (`0xe0`-prefixed) keys are ignored
//! for now.

use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::idt::{self, StackFrame};
use crate::input::{self, KeyEvent};
use crate::pic::{self, IRQ_VECTOR_BASE};

/// IRQ line the keyboard controller drives.
pub const IRQ: u8 = 1;
/// Vector that line raises after [`crate::pic::init`]'s remap.
pub const VECTOR: u8 = IRQ_VECTOR_BASE + IRQ;

/// 8042 data port - read a scancode (or write a command to the keyboard).
const DATA_PORT: u16 = 0x60;
/// 8042 status/command port. Bit 0 set means the output buffer holds a byte.
const STATUS_PORT: u16 = 0x64;

/// Status bit: output buffer full (a byte is waiting to be read from 0x60).
const STATUS_OUTPUT_FULL: u8 = 1 << 0;

/// Scancode set 1 prefix for the second byte of an extended key. Ignored.
const SCANCODE_EXTENDED: u8 = 0xe0;
/// Break-code flag: make code OR'd with this means the key was released.
const SCANCODE_BREAK: u8 = 0x80;

/// Left / right Shift make codes (set 1).
const SCAN_LSHIFT: u8 = 0x2a;
const SCAN_RSHIFT: u8 = 0x36;
/// Enter / Return.
const SCAN_ENTER: u8 = 0x1c;
/// Backspace.
const SCAN_BACKSPACE: u8 = 0x0e;
/// Space bar.
const SCAN_SPACE: u8 = 0x39;

/// Set while the next scancode is the second byte of an `0xe0` sequence.
static EXTENDED: AtomicBool = AtomicBool::new(false);
/// Either Shift key is currently held.
static SHIFT: AtomicBool = AtomicBool::new(false);
/// Make codes translated and enqueued since [`init`].
static ENQUEUED: AtomicU32 = AtomicU32::new(0);

/// Installs the IRQ1 handler and unmasks the line.
///
/// Must run after [`crate::interrupts::init`] (so there is a catch-all to
/// replace on [`VECTOR`]) and after [`crate::pit::init`] is fine - both lines
/// share the master PIC and neither depends on the other.
///
/// Infallible like the other drivers: the controller was already brought up
/// by the firmware, and a kernel that cannot take keyboard IRQs still boots.
pub fn init() {
    EXTENDED.store(false, Ordering::Relaxed);
    SHIFT.store(false, Ordering::Relaxed);
    ENQUEUED.store(0, Ordering::Relaxed);

    // Drain any byte the firmware left sitting in the output buffer so the
    // first IRQ we see is for a real keystroke, not a stale ACK.
    drain_output_buffer();

    // SAFETY: IRQ1 pushes no error code, and `VECTOR` is exactly the one the
    // PIC remap pointed that line at. Replaces the catch-all `interrupts::init`
    // installed for the same vector.
    unsafe {
        idt::set_handler(VECTOR, keyboard_interrupt);
    }

    pic::unmask(IRQ);
}

/// How many keys have been enqueued since [`init`]. Useful for tests;
/// ordinary code does not need it.
pub fn enqueued_count() -> u32 {
    ENQUEUED.load(Ordering::Relaxed)
}

extern "x86-interrupt" fn keyboard_interrupt(_frame: StackFrame) {
    // Always consume the byte if one is present - leaving it in the buffer
    // keeps the line asserted on some controllers and re-fires IRQ1 forever.
    // SAFETY: port 0x60 is the 8042 data port; reading it acknowledges the
    // pending scancode. Only called from the IRQ1 handler, so a byte should
    // be waiting; the status check still guards a spurious edge.
    let status = unsafe { inb(STATUS_PORT) };
    let scancode = if status & STATUS_OUTPUT_FULL != 0 {
        unsafe { inb(DATA_PORT) }
    } else {
        // Spurious IRQ1 with nothing to read - still EOI and return.
        let _ = pic::end_of_interrupt(IRQ);
        return;
    };

    // Acknowledge before translating: the queue push takes IrqMutex, and
    // leaving IRQ1 in service would also block IRQ0 (the timer).
    let _ = pic::end_of_interrupt(IRQ);

    handle_scancode(scancode);
}

fn handle_scancode(scancode: u8) {
    if scancode == SCANCODE_EXTENDED {
        EXTENDED.store(true, Ordering::Relaxed);
        return;
    }

    if EXTENDED.swap(false, Ordering::Relaxed) {
        // Extended make/break (arrows, etc.) - not part of the basic layout.
        return;
    }

    let released = scancode & SCANCODE_BREAK != 0;
    let make = scancode & !SCANCODE_BREAK;

    match make {
        SCAN_LSHIFT | SCAN_RSHIFT => {
            SHIFT.store(!released, Ordering::Relaxed);
            return;
        }
        _ if released => return,
        _ => {}
    }

    if let Some(event) = translate(make) {
        if input::push(event) {
            ENQUEUED.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Set-1 make code → event, applying the current Shift state for letters.
fn translate(make: u8) -> Option<KeyEvent> {
    let shift = SHIFT.load(Ordering::Relaxed);
    Some(match make {
        SCAN_ENTER => KeyEvent::Enter,
        SCAN_BACKSPACE => KeyEvent::Backspace,
        SCAN_SPACE => KeyEvent::Char(' '),
        // Digits, top row (unshifted forms only - shifted symbols are out of
        // scope for the basic layout).
        0x02 => KeyEvent::Char('1'),
        0x03 => KeyEvent::Char('2'),
        0x04 => KeyEvent::Char('3'),
        0x05 => KeyEvent::Char('4'),
        0x06 => KeyEvent::Char('5'),
        0x07 => KeyEvent::Char('6'),
        0x08 => KeyEvent::Char('7'),
        0x09 => KeyEvent::Char('8'),
        0x0a => KeyEvent::Char('9'),
        0x0b => KeyEvent::Char('0'),
        // Letters.
        0x1e => KeyEvent::Char(letter('a', shift)),
        0x30 => KeyEvent::Char(letter('b', shift)),
        0x2e => KeyEvent::Char(letter('c', shift)),
        0x20 => KeyEvent::Char(letter('d', shift)),
        0x12 => KeyEvent::Char(letter('e', shift)),
        0x21 => KeyEvent::Char(letter('f', shift)),
        0x22 => KeyEvent::Char(letter('g', shift)),
        0x23 => KeyEvent::Char(letter('h', shift)),
        0x17 => KeyEvent::Char(letter('i', shift)),
        0x24 => KeyEvent::Char(letter('j', shift)),
        0x25 => KeyEvent::Char(letter('k', shift)),
        0x26 => KeyEvent::Char(letter('l', shift)),
        0x32 => KeyEvent::Char(letter('m', shift)),
        0x31 => KeyEvent::Char(letter('n', shift)),
        0x18 => KeyEvent::Char(letter('o', shift)),
        0x19 => KeyEvent::Char(letter('p', shift)),
        0x10 => KeyEvent::Char(letter('q', shift)),
        0x13 => KeyEvent::Char(letter('r', shift)),
        0x1f => KeyEvent::Char(letter('s', shift)),
        0x14 => KeyEvent::Char(letter('t', shift)),
        0x16 => KeyEvent::Char(letter('u', shift)),
        0x2f => KeyEvent::Char(letter('v', shift)),
        0x11 => KeyEvent::Char(letter('w', shift)),
        0x2d => KeyEvent::Char(letter('x', shift)),
        0x15 => KeyEvent::Char(letter('y', shift)),
        0x2c => KeyEvent::Char(letter('z', shift)),
        _ => return None,
    })
}

fn letter(lower: char, shift: bool) -> char {
    if shift {
        // 'a'..='z' are contiguous in ASCII; uppercase is + ('A' - 'a').
        (lower as u8 - b'a' + b'A') as char
    } else {
        lower
    }
}

/// Reads and discards every byte currently sitting in the 8042 output buffer.
fn drain_output_buffer() {
    // Bound the loop: a stuck status bit must not hang boot.
    for _ in 0..256 {
        // SAFETY: status port read has no side effects; data port read pops
        // one byte from the controller's output buffer.
        let status = unsafe { inb(STATUS_PORT) };
        if status & STATUS_OUTPUT_FULL == 0 {
            break;
        }
        let _ = unsafe { inb(DATA_PORT) };
    }
}

/// Reads a byte from I/O port `port`.
///
/// # Safety
/// As for [`crate::pic`]: reading some ports has side effects on the device.
unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    unsafe {
        asm!(
            "inb %dx, %al",
            in("dx") port,
            out("al") value,
            options(att_syntax, nomem, nostack, preserves_flags),
        );
    }
    value
}
