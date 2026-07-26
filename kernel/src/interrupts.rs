//! Turning interrupt delivery on, and catching the vectors nothing owns yet.
//!
//! Everything the kernel needs before it can survive an interrupt is now in
//! place - a GDT and TSS ([`crate::gdt`], [`crate::tss`]), an IDT with real
//! handlers for the exceptions a kernel bug is likely to trip
//! ([`crate::exceptions`]), and both 8259s remapped above the exception
//! vectors with every line masked ([`crate::pic`]) - so this module does the
//! last step: `sti`.
//!
//! It cannot do that alone, though. [`crate::idt::init`] leaves every vector
//! it isn't asked about "not present", and taking a not-present vector raises a
//! general protection fault *whose own delivery* the CPU counts as a second
//! exception, so one stray interrupt would escalate to a double fault. Which
//! means "interrupts are on" and "every vector goes somewhere" have to arrive
//! together: [`init`] fills every gap with a catch-all that reports what
//! arrived and then either resumes (a stray IRQ, or a software `int` to an
//! unused vector - neither is worth stopping the machine over) or halts (an
//! unhandled CPU exception, which would otherwise re-execute the faulting
//! instruction and fault again forever).
//!
//! Nothing should actually arrive yet: the PIC's masks are all set, so no IRQ
//! can get through until the driver that owns one clears its bit. The
//! catch-all is what makes that safe to get wrong - an unmasked line with no
//! driver behind it prints a diagnostic instead of taking the machine down.

use core::arch::asm;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::diag_println;
use crate::idt::{self, Handler, HandlerWithErrorCode, StackFrame};
use crate::pic::{self, Acknowledged};

/// EFLAGS bit 9, the interrupt flag: set means the CPU will accept a maskable
/// hardware interrupt. `sti` sets it, `cli` clears it, and an interrupt gate
/// clears it for the duration of the handler.
const EFLAGS_INTERRUPT_ENABLE: u32 = 1 << 9;

/// Vectors 0-31 are reserved by Intel for the CPU's own exceptions; anything
/// at or above this is an external interrupt or a software `int n`.
const EXCEPTION_VECTOR_COUNT: u8 = 32;

/// How many interrupts have reached the catch-all, reported or not.
static UNHANDLED_TOTAL: AtomicU32 = AtomicU32::new(0);

/// One bit per vector, recording whether it has been reported already. A line
/// that is unmasked with no driver behind it fires continuously (the PIT would
/// manage 18 times a second), and thousands of identical reports would scroll
/// the first one - the one with useful context in it - off the screen.
static REPORTED: [AtomicU32; 8] = [const { AtomicU32::new(0) }; 8];

/// Installs the catch-all on every vector that has no handler of its own, and
/// reports how many that was.
///
/// Must run *after* [`crate::exceptions::init`], whose handlers are the ones
/// worth keeping: this only fills gaps, it never replaces a registered
/// handler.
pub fn init() -> usize {
    // Written out rather than trusted from the initialisers above: an all-zero
    // static lands in `.bss`, which carries no file data for
    // `objcopy -O binary` to emit, so the flat-binary loader in `boot/boot.asm`
    // never writes those bytes (see `idt::init` for the longer version).
    UNHANDLED_TOTAL.store(0, Ordering::Relaxed);
    for word in REPORTED.iter() {
        word.store(0, Ordering::Relaxed);
    }

    let mut installed = 0;
    for vector in 0..=u8::MAX {
        if idt::has_handler(vector) {
            continue;
        }
        // SAFETY: each vector gets the handler shape the CPU enters it with -
        // `error_code_stub` is exactly the architectural list of vectors that
        // push an error code - and the vector each stub reports is its own
        // index in the table, by construction.
        unsafe {
            match error_code_stub(vector) {
                Some(handler) => idt::set_handler_with_error_code(vector, handler),
                None => idt::set_handler(vector, stub(vector)),
            }
        }
        installed += 1;
    }
    installed
}

/// Lets the CPU start delivering maskable hardware interrupts.
///
/// [`init`] must have run first, and so must [`crate::pic::init`]: before the
/// remap, IRQ0 arrives as vector 8 - the double fault - and the BIOS leaves
/// several lines unmasked, so the first timer tick would look like a fatal
/// fault.
pub fn enable() {
    // SAFETY: every one of the 256 vectors has a present gate pointing at code
    // that either handles it or reports it, so there is no vector left whose
    // delivery would fault. Not `preserves_flags`: changing IF is the point.
    unsafe { asm!("sti", options(att_syntax, nomem, nostack)) };
}

/// Whether the CPU is currently accepting maskable interrupts, read back from
/// EFLAGS rather than remembered - the boot banner reports what the hardware
/// says, not what the kernel asked for.
pub fn enabled() -> bool {
    let eflags: u32;
    // SAFETY: pushes the flags word onto the current stack and pops it into a
    // register; touches no other memory.
    unsafe {
        asm!(
            "pushfl",
            "popl {flags}",
            flags = out(reg) eflags,
            options(att_syntax),
        );
    }
    eflags & EFLAGS_INTERRUPT_ENABLE != 0
}

/// How many interrupts have hit the catch-all so far. Zero is the expected
/// answer for as long as every IRQ line stays masked.
pub fn unhandled_count() -> u32 {
    UNHANDLED_TOTAL.load(Ordering::Relaxed)
}

/// Catch-all for a vector the CPU enters without an error code.
///
/// The vector is a const parameter because an `extern "x86-interrupt"` handler
/// is told nothing about which vector it was entered for - the only way to
/// know is for each vector to have an entry point of its own, which is what
/// [`STUB_ROWS`] monomorphises this into.
extern "x86-interrupt" fn unhandled<const VECTOR: u8>(frame: StackFrame) {
    dispatch(VECTOR, None, &frame);
}

/// Catch-all for a vector the CPU enters with an error code pushed.
extern "x86-interrupt" fn unhandled_with_error_code<const VECTOR: u8>(
    frame: StackFrame,
    error_code: u32,
) {
    dispatch(VECTOR, Some(error_code), &frame);
}

/// Reports an interrupt nothing is registered for, then decides whether the
/// kernel can carry on.
fn dispatch(vector: u8, error_code: Option<u32>, frame: &StackFrame) {
    UNHANDLED_TOTAL.fetch_add(1, Ordering::Relaxed);
    let already_reported = mark_reported(vector);

    // Acknowledge before printing, not after: an IRQ that is still in service
    // blocks its own line and every lower-priority one, and a screenful of
    // diagnostics is long enough for that to matter.
    let irq = pic::irq_of_vector(vector);
    let spurious = irq.map(|irq| pic::end_of_interrupt(irq) == Acknowledged::Spurious);

    if !already_reported {
        diag_println!("");
        match (irq, spurious) {
            (Some(irq), Some(true)) => {
                diag_println!("*** UNHANDLED INTERRUPT #{}: spurious IRQ{}", vector, irq);
            }
            (Some(irq), _) => {
                diag_println!("*** UNHANDLED INTERRUPT #{}: IRQ{}, no driver", vector, irq);
            }
            _ if vector < EXCEPTION_VECTOR_COUNT => {
                diag_println!(
                    "*** UNHANDLED INTERRUPT #{}: {}",
                    vector,
                    exception_name(vector)
                );
            }
            _ => {
                diag_println!("*** UNHANDLED INTERRUPT #{}: no handler", vector);
            }
        }
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
        if vector < EXCEPTION_VECTOR_COUNT {
            diag_println!("    Halted - an unhandled CPU exception cannot be resumed.");
        } else if spurious == Some(true) {
            diag_println!("    Resuming - spurious, so the PIC got no EOI.");
        } else if irq.is_some() {
            diag_println!("    Resuming - the IRQ was acknowledged and dropped.");
        } else {
            diag_println!("    Resuming - nothing can raise this vector but the kernel.");
        }
        diag_println!("    Further reports for this vector are suppressed.");
    }

    // Returning from an exception would re-execute the instruction that
    // faulted, fault again, and print again - forever. There is nothing to
    // retry, so this is the end of the road, the same as a registered
    // exception handler.
    if vector < EXCEPTION_VECTOR_COUNT {
        crate::hlt_loop();
    }
}

/// Records that `vector` has been reported, returning whether it already had
/// been.
fn mark_reported(vector: u8) -> bool {
    let bit = 1u32 << (vector % 32);
    REPORTED[(vector / 32) as usize].fetch_or(bit, Ordering::Relaxed) & bit != 0
}

/// Intel's name for exception `vector`, so an unhandled one says what it is
/// instead of only which number it is. The reserved vectors are named as such
/// deliberately: seeing "reserved" says "this shouldn't be possible", which is
/// itself a useful thing for a crash report to say.
fn exception_name(vector: u8) -> &'static str {
    match vector {
        0 => "divide error",
        1 => "debug",
        2 => "non-maskable interrupt",
        3 => "breakpoint",
        4 => "overflow",
        5 => "bound range exceeded",
        6 => "invalid opcode",
        7 => "device not available",
        8 => "double fault",
        9 => "coprocessor segment overrun",
        10 => "invalid TSS",
        11 => "segment not present",
        12 => "stack-segment fault",
        13 => "general protection fault",
        14 => "page fault",
        16 => "x87 floating-point error",
        17 => "alignment check",
        18 => "machine check",
        19 => "SIMD floating-point exception",
        20 => "virtualization exception",
        21 => "control protection exception",
        28 => "hypervisor injection exception",
        29 => "VMM communication exception",
        30 => "security exception",
        _ => "reserved exception",
    }
}

/// The catch-all entry point for `vector`.
fn stub(vector: u8) -> Handler {
    STUB_ROWS[(vector >> 4) as usize][(vector & 0x0f) as usize]
}

/// The catch-all entry point for `vector`, for the vectors the CPU pushes an
/// error code for. Getting this list wrong is not a cosmetic mistake: a
/// handler of the wrong shape reads the error code where it expects `eip` and
/// leaves the stack four bytes off at `iret`, so it would report nonsense and
/// then return to a garbage address.
fn error_code_stub(vector: u8) -> Option<HandlerWithErrorCode> {
    Some(match vector {
        // Vector 8 in practice goes through a task gate (see `crate::tss`), so
        // this is only reached if that is ever not installed - but the shape is
        // still the correct one for it.
        8 => unhandled_with_error_code::<8>,
        10 => unhandled_with_error_code::<10>,
        11 => unhandled_with_error_code::<11>,
        12 => unhandled_with_error_code::<12>,
        13 => unhandled_with_error_code::<13>,
        14 => unhandled_with_error_code::<14>,
        17 => unhandled_with_error_code::<17>,
        21 => unhandled_with_error_code::<21>,
        29 => unhandled_with_error_code::<29>,
        30 => unhandled_with_error_code::<30>,
        _ => return None,
    })
}

/// Sixteen consecutive catch-all entry points starting at `$base`.
macro_rules! stub_row {
    ($base:expr) => {
        [
            unhandled::<{ $base }>,
            unhandled::<{ $base + 1 }>,
            unhandled::<{ $base + 2 }>,
            unhandled::<{ $base + 3 }>,
            unhandled::<{ $base + 4 }>,
            unhandled::<{ $base + 5 }>,
            unhandled::<{ $base + 6 }>,
            unhandled::<{ $base + 7 }>,
            unhandled::<{ $base + 8 }>,
            unhandled::<{ $base + 9 }>,
            unhandled::<{ $base + 10 }>,
            unhandled::<{ $base + 11 }>,
            unhandled::<{ $base + 12 }>,
            unhandled::<{ $base + 13 }>,
            unhandled::<{ $base + 14 }>,
            unhandled::<{ $base + 15 }>,
        ]
    };
}

/// One entry point per vector, in rows of 16 purely so the table fits on a
/// screen. Indexed by [`stub`]; see [`unhandled`] for why 256 separate
/// functions are needed at all.
static STUB_ROWS: [[Handler; 16]; 16] = [
    stub_row!(0x00),
    stub_row!(0x10),
    stub_row!(0x20),
    stub_row!(0x30),
    stub_row!(0x40),
    stub_row!(0x50),
    stub_row!(0x60),
    stub_row!(0x70),
    stub_row!(0x80),
    stub_row!(0x90),
    stub_row!(0xa0),
    stub_row!(0xb0),
    stub_row!(0xc0),
    stub_row!(0xd0),
    stub_row!(0xe0),
    stub_row!(0xf0),
];

/// Raises one unexpected interrupt on purpose, so the catch-all above is
/// verified against a real one rather than by inspection - the same
/// compile-time-feature trick [`crate::exceptions::trigger_debug_exception`]
/// uses, e.g. `make run KERNEL_FEATURES=trigger-unhandled-interrupt`. A normal
/// build enables neither feature and compiles this to nothing.
pub fn trigger_debug_interrupt() {
    #[cfg(feature = "trigger-unhandled-interrupt")]
    {
        diag_println!("DEBUG: taking vector 0x60, which nothing has registered...");
        // SAFETY: 0x60 is outside both the exception range and the PIC's, so
        // the only thing that can raise it is this instruction, and the
        // catch-all it reaches returns rather than halting - which is the
        // property being tested, so the line printed afterwards is the proof.
        unsafe { asm!("int $0x60", options(att_syntax)) };
        diag_println!(
            "DEBUG: resumed after the report ({} unhandled so far)",
            unhandled_count()
        );
    }

    #[cfg(feature = "trigger-spurious-irq")]
    {
        diag_println!("DEBUG: taking vector 39 (IRQ7), the spurious-interrupt line...");
        // A real spurious IRQ needs a line that drops before the CPU
        // acknowledges it, which software cannot arrange - but the handler's
        // decision is made from the PIC's in-service register, and IRQ7 is not
        // in service here, so a software `int` reaches exactly the same branch
        // a genuine phantom interrupt would.
        //
        // SAFETY: as above - vector 39 has the catch-all installed, and it
        // returns.
        unsafe { asm!("int $0x27", options(att_syntax)) };
        diag_println!(
            "DEBUG: resumed after the report ({} unhandled so far)",
            unhandled_count()
        );
    }
}
