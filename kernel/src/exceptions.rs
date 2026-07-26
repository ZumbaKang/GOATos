//! Handlers for the CPU exceptions worth reporting today.
//!
//! With an IDT loaded but every vector "not present" (see [`crate::idt`]), a
//! kernel bug that trips an exception takes a vector the CPU can't dispatch,
//! which escalates to a double fault and - with no handler for that either -
//! a triple fault: the machine resets, printing nothing. This module turns
//! the three exceptions most likely to be hit by ordinary kernel bugs into a
//! readable report instead:
//!
//! - divide error (0) - `div`/`idiv` with a zero divisor, or a quotient too
//!   large for its destination,
//! - invalid opcode (6) - execution ran into something that isn't an
//!   instruction, the usual symptom of a jump to a bad address or of
//!   overwritten code,
//! - general protection fault (13) - a catch-all for illegal segment,
//!   privilege, and descriptor-table use; also what an unregistered vector
//!   raises, so it covers "took an interrupt nothing handles" too.
//!
//! There is no recovery: a handler prints and halts. Anything cleverer needs
//! state this kernel doesn't have yet, and a wrong guess about whether it is
//! safe to resume turns one readable crash into an unreadable one.
//!
//! Double fault (8) is deliberately *not* here - it needs its own stack via a
//! TSS to survive a stack overflow, which is its own roadmap task.
//!
//! Known limitation: reporting goes through the VGA writer's lock, so an
//! exception raised *while that lock is held* (i.e. from inside a print)
//! deadlocks instead of reporting. Nothing on the current code paths prints
//! from interrupt context, so this stays a latent concern rather than a live
//! one.

use crate::idt::{self, StackFrame};

/// Prints to both output surfaces: VGA because it is the primary display (and
/// the only one a browser visitor or a real monitor sees), serial because it
/// is the one a headless QEMU and CI can read.
macro_rules! diag_println {
    ($($arg:tt)*) => {{
        $crate::vga_println!($($arg)*);
        $crate::serial_println!($($arg)*);
    }};
}

/// Divide error - no error code.
const DIVIDE_ERROR: u8 = 0;
/// Invalid opcode - no error code.
const INVALID_OPCODE: u8 = 6;
/// General protection fault - pushes an error code, which for a
/// segment-related fault is the offending selector (and 0 otherwise).
const GENERAL_PROTECTION_FAULT: u8 = 13;

/// One-line summary of what [`init`] installed, for the boot banner.
/// `scripts/ci-test.sh` greps for it, so printing it doubles as the automated
/// check that these handlers are still being registered.
pub const INSTALLED_SUMMARY: &str = "Exceptions: #0 divide error, #6 invalid opcode, #13 GP fault";

/// Registers the handlers above. [`crate::idt::init`] must have run first (it
/// resets every vector) and [`crate::gdt::init`] before that (the gates refer
/// to the kernel code selector).
pub fn init() {
    // SAFETY: each vector is paired with the handler shape the CPU uses for
    // it - 0 and 6 push no error code, 13 does - which is the one thing
    // `set_handler`/`set_handler_with_error_code` can't check themselves.
    unsafe {
        idt::set_handler(DIVIDE_ERROR, divide_error);
        idt::set_handler(INVALID_OPCODE, invalid_opcode);
        idt::set_handler_with_error_code(GENERAL_PROTECTION_FAULT, general_protection_fault);
    }
}

extern "x86-interrupt" fn divide_error(frame: StackFrame) {
    report(DIVIDE_ERROR, "divide error", None, &frame);
}

extern "x86-interrupt" fn invalid_opcode(frame: StackFrame) {
    report(INVALID_OPCODE, "invalid opcode", None, &frame);
}

extern "x86-interrupt" fn general_protection_fault(frame: StackFrame, error_code: u32) {
    report(
        GENERAL_PROTECTION_FAULT,
        "general protection fault",
        Some(error_code),
        &frame,
    );
}

/// Prints what the CPU reported about the exception, then halts for good.
///
/// `eip` is the faulting instruction itself for all three vectors here (they
/// are faults, not traps), so it can be matched straight against
/// `objdump -d kernel/target/i686-goatos/debug/kernel` - the kernel is loaded
/// at the address the linker script gives it, so no rebasing is needed.
fn report(vector: u8, name: &str, error_code: Option<u32>, frame: &StackFrame) -> ! {
    diag_println!("");
    diag_println!("*** CPU EXCEPTION #{}: {}", vector, name);
    match error_code {
        Some(code) => diag_println!(
            "    eip={:#010x} cs={:#06x} eflags={:#010x} error={:#010x}",
            frame.eip,
            frame.cs,
            frame.eflags,
            code
        ),
        None => diag_println!(
            "    eip={:#010x} cs={:#06x} eflags={:#010x}",
            frame.eip,
            frame.cs,
            frame.eflags
        ),
    }
    diag_println!("    Halted - this exception is not recoverable.");

    crate::hlt_loop()
}

/// Raises one CPU exception on purpose, so the handlers above can be verified
/// end to end rather than by inspection.
///
/// Which one is chosen at compile time by a `trigger-*` cargo feature, e.g.
/// `make disk KERNEL_FEATURES=trigger-divide-error`; a normal build enables
/// none of them and compiles this to nothing. Enabling more than one is
/// pointless rather than harmful - the first to fault halts the kernel.
pub fn trigger_debug_exception() {
    #[cfg(feature = "trigger-divide-error")]
    {
        diag_println!("DEBUG: dividing by zero on purpose...");
        // SAFETY: raising the exception *is* the point, and `divide_error`
        // halts, so no instruction after this - and none of the clobbered
        // registers - is ever reached.
        unsafe {
            core::arch::asm!(
                "movl $1, %eax",
                "xorl %edx, %edx",
                "xorl %ecx, %ecx",
                "divl %ecx",
                out("eax") _,
                out("edx") _,
                out("ecx") _,
                options(att_syntax, nostack),
            );
        }
    }

    #[cfg(feature = "trigger-invalid-opcode")]
    {
        diag_println!("DEBUG: executing a reserved instruction on purpose...");
        // SAFETY: `ud2` is the architecturally-guaranteed way to raise #UD;
        // `invalid_opcode` halts, so execution does not continue past it.
        unsafe { core::arch::asm!("ud2", options(att_syntax, nostack)) };
    }

    #[cfg(feature = "trigger-general-protection-fault")]
    {
        diag_println!("DEBUG: loading an out-of-range segment selector on purpose...");
        // SAFETY: selector 0x38 is past the end of the kernel's three-entry
        // GDT, so loading it raises #GP with the selector as its error code -
        // which also exercises the error-code path. DS is left untouched
        // because the load faults instead of taking effect, and
        // `general_protection_fault` halts regardless.
        unsafe {
            core::arch::asm!(
                "movw $0x38, %ax",
                "movw %ax, %ds",
                out("eax") _,
                options(att_syntax, nostack),
            );
        }
    }
}
