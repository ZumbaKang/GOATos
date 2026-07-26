//! Kernel Interrupt Descriptor Table.
//!
//! The IDT is the table the CPU consults to find out where to jump when
//! *anything* interrupts normal execution: a CPU exception (divide error,
//! general protection fault, ...), a hardware IRQ, or a software `int n`.
//! Until it exists, any such event has no defined destination, which is why
//! a kernel bug currently manifests as a silent freeze or reboot.
//!
//! This module is only the scaffolding for that: the 256-entry table, a way
//! to register a handler for a vector, and the `lidt` that hands the table to
//! the CPU. It deliberately installs *no* handlers - [`init`] leaves every
//! vector "not present" - so what the table does is entirely up to its
//! callers: [`crate::exceptions`] registers the faults worth reporting
//! individually, and [`crate::interrupts`] fills the remaining gaps with a
//! catch-all before enabling interrupts, since a not-present vector is one
//! stray interrupt away from a double fault.

use core::arch::asm;
use core::cell::UnsafeCell;
use core::mem::size_of;

use crate::gdt;

/// Vectors are 8-bit, so the table tops out at 256 entries.
pub const ENTRY_COUNT: usize = 256;

/// Present bit of a gate descriptor's type/attribute byte. Its absence is
/// what makes an unregistered vector fault instead of jumping into whatever
/// bytes happen to be at offset 0.
const PRESENT: u8 = 0b1000_0000;
/// Gate type nibble for a 32-bit interrupt gate: like a trap gate (0xf), but
/// it also clears IF on entry, so a handler can't be interrupted by the same
/// IRQ it is still servicing. Everything this kernel will register - CPU
/// exceptions and PIC IRQs alike - wants that, so trap gates aren't offered
/// until something actually needs one.
const GATE_32BIT_INTERRUPT: u8 = 0xe;
/// Gate type nibble for a task gate: instead of jumping to an offset in the
/// current task, the CPU performs a full hardware task switch to the TSS the
/// gate names - which is the only way, on 32-bit x86, for a handler to run on
/// a stack of its own (see [`crate::tss`]).
const GATE_TASK: u8 = 0x5;

/// The registers the CPU pushes before entering a handler, in the order a
/// handler sees them (lowest address first).
///
/// This is the same-privilege-level layout. An interrupt that crosses rings
/// also pushes the interrupted `esp`/`ss` above `eflags`, which can't happen
/// yet - there is no ring 3, and no separate stack per privilege level.
///
/// Handlers must take this **by value**, not as a pointer: an
/// `extern "x86-interrupt"` parameter is materialised from the interrupt
/// frame itself, so a pointer parameter is read *out of* the frame (yielding
/// the interrupted `eip`) instead of pointing *at* it. Passing the struct by
/// value makes it indirect, which is what lines the two up.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct StackFrame {
    /// Address of the interrupted (or faulting) instruction.
    pub eip: u32,
    /// Code segment selector it was running under.
    pub cs: u32,
    /// Flags at the point of interruption.
    pub eflags: u32,
}

/// A handler for a vector that pushes no error code.
///
/// `extern "x86-interrupt"` is what makes this usable as a raw entry point:
/// the compiler emits a prologue that preserves every register the handler
/// touches and an `iret` epilogue, instead of the ordinary `ret` that would
/// leave the interrupt frame on the stack.
pub type Handler = extern "x86-interrupt" fn(StackFrame);

/// A handler for a vector that pushes an error code - on 32-bit x86: double
/// fault (8), invalid TSS (10), segment-not-present (11), stack-segment fault
/// (12), general protection fault (13), page fault (14), alignment check
/// (17), control protection (21).
pub type HandlerWithErrorCode = extern "x86-interrupt" fn(StackFrame, u32);

/// One 8-byte IDT gate. Unlike a GDT descriptor, the fields are byte-aligned
/// rather than scattered across the quadword, so a plain `repr(C)` struct
/// describes it exactly.
#[derive(Clone, Copy)]
#[repr(C)]
struct Gate {
    offset_low: u16,
    selector: u16,
    reserved: u8,
    type_attr: u8,
    offset_high: u16,
}

impl Gate {
    /// A vector with no handler: not present, so taking it raises a
    /// general protection fault rather than jumping somewhere arbitrary.
    const MISSING: Gate = Gate {
        offset_low: 0,
        selector: 0,
        reserved: 0,
        type_attr: 0,
        offset_high: 0,
    };

    /// A ring-0 32-bit interrupt gate pointing at `offset` within `selector`.
    fn interrupt(offset: u32, selector: u16) -> Gate {
        Gate {
            offset_low: offset as u16,
            selector,
            reserved: 0,
            // DPL stays 0: only the CPU (and ring-0 code) may take these
            // vectors, not a future ring-3 `int n`.
            type_attr: PRESENT | GATE_32BIT_INTERRUPT,
            offset_high: (offset >> 16) as u16,
        }
    }

    /// A ring-0 task gate naming `tss_selector`. The offset fields go unused:
    /// where execution resumes comes from the TSS's saved `eip`, not from the
    /// gate.
    fn task(tss_selector: u16) -> Gate {
        Gate {
            offset_low: 0,
            selector: tss_selector,
            reserved: 0,
            type_attr: PRESENT | GATE_TASK,
            offset_high: 0,
        }
    }

    fn is_present(&self) -> bool {
        self.type_attr & PRESENT != 0
    }
}

/// 8-byte aligned because the CPU reads gates as 8-byte quantities.
#[repr(C, align(8))]
struct Idt {
    entries: [Gate; ENTRY_COUNT],
}

/// The IDT has to stay writable - handlers are registered at runtime - so,
/// unlike the GDT, it can't be a plain immutable `static` in `.rodata`.
/// Wrapping it in an `UnsafeCell` keeps every mutation an explicit,
/// individually-justified `unsafe` block instead of a `static mut`.
struct IdtCell(UnsafeCell<Idt>);

// SAFETY: GOATos is single-CPU and single-threaded, and the table is only
// mutated with interrupts masked, so there is no concurrent access to
// synchronise. Revisit if either of those stops being true (SMP, or
// registering a handler while interrupts are live).
unsafe impl Sync for IdtCell {}

static IDT: IdtCell = IdtCell(UnsafeCell::new(Idt {
    entries: [Gate::MISSING; ENTRY_COUNT],
}));

/// The operand `lidt`/`sidt` take: a 16-bit limit followed by a 32-bit base.
#[derive(Clone, Copy, Default)]
#[repr(C, packed)]
struct IdtPointer {
    limit: u16,
    base: u32,
}

impl IdtPointer {
    fn for_table(idt: *const Idt) -> IdtPointer {
        IdtPointer {
            // The limit is the *last valid byte's* offset, not the size.
            limit: (size_of::<Idt>() - 1) as u16,
            base: idt as u32,
        }
    }
}

/// What the CPU currently has loaded, read back with `sidt`, plus how much of
/// it is actually wired up - enough to show the kernel's own table is in
/// effect and how far the interrupt work has got.
pub struct Loaded {
    pub base: u32,
    pub limit: u16,
    /// Vectors with a handler registered.
    pub handlers: usize,
}

impl Loaded {
    /// Entry count implied by the limit the CPU reported.
    pub fn entries(&self) -> usize {
        (self.limit as usize + 1) / size_of::<Gate>()
    }
}

/// Builds an empty IDT and loads it, replacing whatever interrupt table state
/// the CPU was left in by the BIOS and `boot/boot.asm`.
///
/// Must run before any handler is registered: it resets every vector, so an
/// earlier registration would be discarded.
pub fn init() {
    // SAFETY: single-threaded, interrupts masked, and no handler can be
    // running yet - nothing else can observe the table mid-reset.
    let idt = unsafe { &mut *IDT.0.get() };

    // Write "not present" out explicitly rather than trusting the static's
    // zero initialiser: an all-zero static lands in `.bss`, and `.bss` holds
    // no file data for `objcopy -O binary` to emit, so the flat-binary loader
    // in `boot/boot.asm` never writes those bytes. They are only zero because
    // the machine happens to start with zeroed RAM - not a property to hang
    // the interrupt table on.
    for entry in idt.entries.iter_mut() {
        *entry = Gate::MISSING;
    }

    let pointer = IdtPointer::for_table(idt);
    // SAFETY: `pointer` describes `IDT`, a correctly-sized, 8-byte-aligned,
    // `'static` table whose every entry was just initialised above.
    unsafe { load(&pointer) };
}

/// Registers `handler` for `vector`, replacing any previous one.
///
/// # Safety
/// `vector` must be one the CPU enters *without* pushing an error code - use
/// [`set_handler_with_error_code`] for the ones that do. Getting this
/// backwards leaves the stack misaligned by four bytes at `iret`, which
/// returns to a garbage address instead of reporting anything.
///
/// [`init`] must have run first, or the registration is discarded by the
/// reset it performs.
pub unsafe fn set_handler(vector: u8, handler: Handler) {
    unsafe { set_gate(vector, handler as usize) };
}

/// Registers `handler` for `vector`, replacing any previous one.
///
/// # Safety
/// `vector` must be one the CPU enters *with* an error code pushed (see
/// [`HandlerWithErrorCode`]); the same stack-misalignment hazard as
/// [`set_handler`] applies in reverse.
pub unsafe fn set_handler_with_error_code(vector: u8, handler: HandlerWithErrorCode) {
    unsafe { set_gate(vector, handler as usize) };
}

/// Routes `vector` through a hardware task switch to `tss_selector` instead of
/// calling a handler on the interrupted stack.
///
/// Used for the double fault (see [`crate::tss`]), where the interrupted stack
/// is exactly what cannot be trusted.
///
/// # Safety
/// `tss_selector` must name an available 32-bit TSS descriptor in the GDT whose
/// TSS is fully populated - `eip`, `esp`, `cs`, `ss` and the data selectors all
/// have to describe a runnable task, because the CPU loads them wholesale and
/// starts executing. The task register must also already point at a *different*
/// valid TSS for the outgoing register set to be saved into, or the switch
/// itself faults.
///
/// [`init`] must have run first, or the registration is discarded by the reset
/// it performs.
pub unsafe fn set_task_gate(vector: u8, tss_selector: u16) {
    // SAFETY: as for `init` - single-threaded, interrupts masked.
    let idt = unsafe { &mut *IDT.0.get() };
    idt.entries[vector as usize] = Gate::task(tss_selector);
}

/// Whether `vector` currently has anything registered for it - a handler
/// function or a task gate.
///
/// Lets [`crate::interrupts`] fill only the vectors nothing owns yet, without
/// having to know which ones [`crate::exceptions`] claimed.
pub fn has_handler(vector: u8) -> bool {
    // SAFETY: as for `init` - single-threaded, and a read of one gate cannot
    // observe a torn write when nothing is writing.
    let idt = unsafe { &*IDT.0.get() };
    idt.entries[vector as usize].is_present()
}

/// Removes any handler for `vector`, leaving it "not present" as [`init`] found
/// it.
///
/// Taking an exception with no handler installed raises a general protection
/// fault instead of dispatching - and if the original exception was itself a
/// contributory one, that escalates to a double fault, which is how
/// [`crate::exceptions`]'s double-fault trigger provokes a real one.
///
/// # Safety
/// The caller must be prepared for `vector` to stop being handled, which for
/// most vectors means the next occurrence takes down the kernel.
pub unsafe fn clear_handler(vector: u8) {
    // SAFETY: as for `init` - single-threaded, interrupts masked.
    let idt = unsafe { &mut *IDT.0.get() };
    idt.entries[vector as usize] = Gate::MISSING;
}

/// # Safety
/// `offset` must be the entry point of a function that returns with `iret`
/// and whose signature matches what the CPU pushes for `vector`.
unsafe fn set_gate(vector: u8, offset: usize) {
    // SAFETY: as for `init` - single-threaded, interrupts masked.
    let idt = unsafe { &mut *IDT.0.get() };
    // Handlers live in the kernel image, reached through the kernel code
    // segment the GDT defines, which is why `gdt::init` has to run first.
    idt.entries[vector as usize] = Gate::interrupt(offset as u32, gdt::KERNEL_CODE_SELECTOR);
}

/// # Safety
/// `pointer` must describe a fully-initialised IDT that stays resident for as
/// long as the CPU can take an interrupt - the CPU reads it out of memory on
/// every exception/IRQ, long after this returns.
unsafe fn load(pointer: &IdtPointer) {
    unsafe {
        asm!(
            "lidt ({idtr})",
            idtr = in(reg) pointer,
            options(att_syntax, nostack, preserves_flags),
        );
    }
}

/// Reads back the IDT the CPU is currently using.
pub fn loaded() -> Loaded {
    let mut pointer = IdtPointer::default();
    // SAFETY: `sidt` only writes the 6 bytes of the pointer we hand it.
    unsafe {
        asm!(
            "sidt ({idtr})",
            idtr = in(reg) &mut pointer,
            options(att_syntax, nostack, preserves_flags),
        );
    }

    // SAFETY: as for `init`.
    let idt = unsafe { &*IDT.0.get() };
    Loaded {
        base: pointer.base,
        limit: pointer.limit,
        handlers: idt.entries.iter().filter(|entry| entry.is_present()).count(),
    }
}
