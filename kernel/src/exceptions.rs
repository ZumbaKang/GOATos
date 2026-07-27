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
//!   privilege, and descriptor-table use, and also what a vector with no gate
//!   at all raises (which is why [`crate::interrupts`] leaves none).
//! - double fault (8) - the CPU could not deliver one of the above. This one
//!   is special: it runs as a separate hardware task so that it has a stack of
//!   its own (see [`crate::tss`]), because the whole reason a double fault
//!   happens is that something about the interrupted context - possibly its
//!   stack - is too broken to take an exception on.
//!
//! There is no recovery: a handler prints and halts. Anything cleverer needs
//! state this kernel doesn't have yet, and a wrong guess about whether it is
//! safe to resume turns one readable crash into an unreadable one.
//!
//! Known limitation: reporting goes through the VGA writer's lock, so an
//! exception raised *while that lock is held* (i.e. from inside a print)
//! deadlocks instead of reporting. Interrupts are enabled now, which makes
//! that reachable in principle - but every IRQ line is still masked, so
//! nothing can interrupt a print. The first driver to unmask a line is what
//! makes it real; see `ROADMAP.md` task 3.0.

use core::arch::asm;

use crate::gdt;
use crate::idt::{self, StackFrame};
use crate::tss;

/// Prints to both output surfaces: VGA because it is the primary display (and
/// the only one a browser visitor or a real monitor sees), serial because it
/// is the one a headless QEMU and CI can read.
///
/// Exported for the other diagnostic path, [`crate::interrupts`]: every crash
/// or "this should not have happened" report wants both surfaces, and wants
/// them in this order.
#[macro_export]
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
/// Double fault - dispatched through a task gate, not a handler function.
const DOUBLE_FAULT: u8 = 8;
/// General protection fault - pushes an error code, which for a
/// segment-related fault is the offending selector (and 0 otherwise).
const GENERAL_PROTECTION_FAULT: u8 = 13;

/// One-line summary of what [`init`] installed, for the boot banner.
/// `scripts/ci-test.sh` greps for it, so printing it doubles as the automated
/// check that these handlers are still being registered.
pub const INSTALLED_SUMMARY: &str =
    "Exceptions: #0 divide error, #6 invalid opcode, #8 double fault, #13 GP fault";

/// Registers the handlers above. [`crate::idt::init`] must have run first (it
/// resets every vector) and [`crate::gdt::init`] before that (the gates refer
/// to the kernel code selector, and to the TSS the double-fault task gate
/// switches to).
pub fn init() {
    // SAFETY: each vector is paired with the handler shape the CPU uses for
    // it - 0 and 6 push no error code, 13 does - which is the one thing
    // `set_handler`/`set_handler_with_error_code` can't check themselves.
    unsafe {
        idt::set_handler(DIVIDE_ERROR, divide_error);
        idt::set_handler(INVALID_OPCODE, invalid_opcode);
        idt::set_handler_with_error_code(GENERAL_PROTECTION_FAULT, general_protection_fault);
    }
    // SAFETY: `gdt::DOUBLE_FAULT_TSS_SELECTOR` names the available 32-bit TSS
    // descriptor `gdt::init` built for the task `tss::init` populated, and
    // `gdt::init` has already pointed the task register at the *other* TSS for
    // the outgoing register set.
    unsafe {
        idt::set_task_gate(DOUBLE_FAULT, gdt::DOUBLE_FAULT_TSS_SELECTOR);
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

/// Where the CPU starts executing after the vector-8 task gate switches tasks.
///
/// This is *not* an `extern "x86-interrupt"` handler: it is a task's entry
/// point, so there is no interrupt frame on the stack (the interrupted
/// registers were saved into the main TSS instead, which is where the report
/// below reads them from) and no `iret` to be emitted - it must never return.
/// The double fault's error code, always zero, is the one thing the CPU does
/// push onto the new stack, and is of no interest.
///
/// Because the CPU loaded `esp` from the double-fault TSS, this runs on a
/// private stack: whatever was wrong with the interrupted one - overflowed,
/// pointing at unwritable memory, misaligned - cannot stop the report.
pub extern "C" fn double_fault_entry() -> ! {
    let handler_esp: u32;
    // SAFETY: reads a register; touches no memory.
    unsafe {
        asm!("movl %esp, {esp}", esp = out(reg) handler_esp, options(att_syntax, nostack, preserves_flags));
    }

    let interrupted = tss::interrupted_state();
    let (stack_bottom, stack_top) = tss::double_fault_stack_range();

    diag_println!("");
    diag_println!("*** CPU EXCEPTION #{}: {}", DOUBLE_FAULT, "double fault");
    diag_println!(
        "    interrupted: eip={:#010x} cs={:#06x} eflags={:#010x}",
        interrupted.eip,
        interrupted.cs,
        interrupted.eflags
    );
    diag_println!(
        "    interrupted stack: esp={:#010x} ebp={:#010x} ss={:#06x}",
        interrupted.esp,
        interrupted.ebp,
        interrupted.ss
    );
    diag_println!(
        "    handler stack: esp={:#010x} in private {:#010x}..{:#010x}",
        handler_esp,
        stack_bottom,
        stack_top
    );
    diag_println!(
        "    switched from TSS {:#06x} via task gate",
        interrupted.previous_task_link
    );
    diag_println!("    Halted - this exception is not recoverable.");

    crate::hlt_loop()
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
            asm!(
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
        unsafe { asm!("ud2", options(att_syntax, nostack)) };
    }

    #[cfg(feature = "trigger-double-fault")]
    {
        diag_println!("DEBUG: unregistering the divide-error handler, then dividing by zero,");
        diag_println!("DEBUG: so the CPU faults while trying to deliver the fault...");
        // A double fault is not something a kernel can ask for directly: it is
        // what the CPU reports when it fails to deliver *another* exception.
        // Taking away #0's handler manufactures exactly that - the divide error
        // below can't be dispatched, which raises a general protection fault
        // during delivery, and two "contributory" exceptions in a row are the
        // architectural definition of a double fault.
        //
        // A stack overflow is the textbook cause; `trigger-stack-overflow`
        // covers that once the guard page from roadmap 2.6 is in place. This
        // path stays as the "fault during delivery" shape, which does not
        // depend on paging.
        //
        // SAFETY: leaving #0 unhandled is the point, and the double-fault task
        // halts, so nothing after this runs.
        unsafe { idt::clear_handler(DIVIDE_ERROR) };
        // SAFETY: as for the `trigger-divide-error` case above.
        unsafe {
            asm!(
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

    #[cfg(feature = "trigger-double-fault-gate")]
    {
        diag_println!("DEBUG: taking vector 8 directly, to exercise the task gate...");
        // Not a real double fault - a software interrupt through the same gate.
        // It proves the task switch and the private stack, which is all that
        // can be shown under emulators that don't implement escalating a
        // failed exception delivery into a double fault (v86 aborts instead).
        // The saved `eip` is therefore the instruction *after* the `int`, not a
        // faulting one.
        //
        // SAFETY: vector 8's task gate is installed by `init`, and the task it
        // switches to halts, so nothing after this runs.
        unsafe { asm!("int $0x08", options(att_syntax)) };
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
            asm!(
                "movw $0x38, %ax",
                "movw %ax, %ds",
                out("eax") _,
                options(att_syntax, nostack),
            );
        }
    }

    #[cfg(feature = "trigger-stack-overflow")]
    {
        diag_println!("DEBUG: overflowing the kernel stack on purpose...");
        diag_println!("DEBUG: (esp should grow into the unmapped guard page and double-fault)");
        // Each frame burns stack with a volatile write so LLVM cannot turn the
        // recursion into a jump. `inline(never)` keeps the call from being
        // flattened away. When `esp` lands in the guard page, pushing the #PF
        // frame fails and the CPU escalates to a double fault on the private
        // stack - which is the whole point of roadmap 2.6.
        #[inline(never)]
        #[allow(unconditional_recursion)] // overflowing the stack is the point
        fn blow_stack() -> ! {
            let mut pad = [0u8; 256];
            // SAFETY: writing into our own stack-local buffer.
            unsafe {
                core::ptr::write_volatile(pad.as_mut_ptr(), 0xaa);
            }
            // Keep `pad` live across the recursive call.
            core::hint::black_box(&pad);
            blow_stack();
        }
        blow_stack();
    }
}
