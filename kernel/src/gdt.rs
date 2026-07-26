//! Kernel-owned Global Descriptor Table.
//!
//! `boot/boot.asm` builds a bare-minimum flat GDT purely to get the CPU into
//! 32-bit protected mode. That table lives in the boot sector's own memory
//! (around 0x7c00), which the kernel is otherwise free to reuse - so the
//! kernel keeps running on descriptors it neither owns nor can extend. This
//! module rebuilds the same flat, ring-0, 4 GiB code/data layout inside the
//! kernel image and loads it, so segmentation is owned by Rust code that
//! later phases can grow (a TSS for the double-fault stack, ring 3
//! segments, ...).
//!
//! Nothing about the *effective* addressing changes: both tables describe
//! base 0, limit 4 GiB segments, so protected-mode addresses stay plain
//! physical addresses across the switch.
//!
//! Beyond the three flat segments, the table also holds descriptors for the
//! two Task State Segments in [`crate::tss`] - the machinery that gives the
//! double-fault handler a stack of its own. Those descriptors have to be built
//! at runtime (a `static`'s address isn't a constant Rust can turn into an
//! integer), and the CPU writes each one's "busy" bit as it switches tasks, so
//! the table lives in an [`UnsafeCell`] rather than being an immutable
//! `static` in `.rodata` as it was before.

use core::arch::asm;
use core::cell::UnsafeCell;
use core::mem::size_of;

use crate::tss;

/// Access byte for a ring-0, present, executable + readable code segment.
const ACCESS_KERNEL_CODE: u8 = 0b1001_1010;
/// Access byte for a ring-0, present, writable data segment.
const ACCESS_KERNEL_DATA: u8 = 0b1001_0010;
/// Access byte for a present, ring-0, *available* 32-bit TSS. Unlike the
/// segment descriptors above this is a system descriptor (S = 0), and the CPU
/// flips its type from "available" (9) to "busy" (11) while the task is
/// running.
const ACCESS_TSS_AVAILABLE: u8 = 0b1000_1001;
/// Flags nibble: 4 KiB granularity + 32-bit segment.
const FLAGS_32BIT_4K: u8 = 0b1100;
/// Flags nibble for a TSS: byte granularity, so the limit can describe the
/// TSS's exact 104-byte length, and none of the size/granularity bits a code
/// or data segment needs.
const FLAGS_TSS: u8 = 0b0000;

/// One 8-byte GDT entry, in the CPU's scattered bit layout.
#[derive(Clone, Copy)]
#[repr(transparent)]
struct Descriptor(u64);

impl Descriptor {
    const NULL: Descriptor = Descriptor(0);

    const fn new(base: u32, limit: u32, access: u8, flags: u8) -> Descriptor {
        let base = base as u64;
        let limit = limit as u64;
        Descriptor(
            (limit & 0xffff)
                | ((base & 0xffff) << 16)
                | (((base >> 16) & 0xff) << 32)
                | ((access as u64) << 40)
                | (((limit >> 16) & 0xf) << 48)
                | (((flags & 0xf) as u64) << 52)
                | (((base >> 24) & 0xff) << 56),
        )
    }

    /// A segment covering the entire 4 GiB address space: base 0, and a limit
    /// of 0xfffff *pages* (4 KiB granularity).
    const fn flat(access: u8) -> Descriptor {
        Descriptor::new(0, 0xfffff, access, FLAGS_32BIT_4K)
    }

    /// A descriptor for a Task State Segment at `base`, `limit` bytes long
    /// minus one - the CPU reads the TSS through this, and writes its busy bit
    /// back into it.
    const fn tss(location: &tss::Location) -> Descriptor {
        Descriptor::new(
            location.base,
            location.limit,
            ACCESS_TSS_AVAILABLE,
            FLAGS_TSS,
        )
    }
}

const NULL_INDEX: usize = 0;
const KERNEL_CODE_INDEX: usize = 1;
const KERNEL_DATA_INDEX: usize = 2;
const MAIN_TSS_INDEX: usize = 3;
const DOUBLE_FAULT_TSS_INDEX: usize = 4;
const ENTRY_COUNT: usize = 5;

/// The table itself. 8-byte aligned because the CPU reads entries as 8-byte
/// quantities, and a `static` so its address stays valid for the whole life
/// of the kernel (the CPU keeps reading it out of memory long after `init`).
#[repr(C, align(8))]
struct Gdt {
    entries: [Descriptor; ENTRY_COUNT],
}

/// The GDT has to stay writable: the TSS descriptors are built at runtime, and
/// the CPU itself sets a TSS descriptor's busy bit on `ltr` and on every task
/// switch. `UnsafeCell` keeps each mutation an explicit `unsafe` block instead
/// of a `static mut`, the same way [`crate::idt`] handles the IDT.
struct GdtCell(UnsafeCell<Gdt>);

// SAFETY: GOATos is single-CPU and single-threaded, and the table is only
// mutated by `init` before any task switch is possible. Revisit for SMP.
unsafe impl Sync for GdtCell {}

static GDT: GdtCell = GdtCell(UnsafeCell::new(Gdt {
    entries: [Descriptor::NULL; ENTRY_COUNT],
}));

/// Selector (byte offset into the GDT) for the kernel code segment.
pub const KERNEL_CODE_SELECTOR: u16 = (KERNEL_CODE_INDEX * 8) as u16;
/// Selector (byte offset into the GDT) for the kernel data segment.
pub const KERNEL_DATA_SELECTOR: u16 = (KERNEL_DATA_INDEX * 8) as u16;
/// Selector for the TSS the CPU saves the interrupted kernel's registers into.
pub const MAIN_TSS_SELECTOR: u16 = (MAIN_TSS_INDEX * 8) as u16;
/// Selector for the double-fault task's TSS, which the IDT's vector-8 task
/// gate names.
pub const DOUBLE_FAULT_TSS_SELECTOR: u16 = (DOUBLE_FAULT_TSS_INDEX * 8) as u16;

// `load` has to spell these selectors out as assembly immediates, so keep the
// two definitions from drifting apart.
const _: () = assert!(KERNEL_CODE_SELECTOR == 0x08);
const _: () = assert!(KERNEL_DATA_SELECTOR == 0x10);

/// The operand `lgdt`/`sgdt` take: a 16-bit limit followed by a 32-bit base.
#[derive(Clone, Copy, Default)]
#[repr(C, packed)]
struct GdtPointer {
    limit: u16,
    base: u32,
}

/// What the CPU currently has loaded, read back with `sgdt`/`str` plus the
/// segment registers - enough to prove which GDT is actually in effect.
pub struct Loaded {
    pub base: u32,
    pub limit: u16,
    pub cs: u16,
    pub ds: u16,
    /// Task register: the selector of the TSS the CPU would save the current
    /// register set into on a task switch. Zero means no TSS is loaded, and a
    /// task gate would fault instead of switching.
    pub tr: u16,
}

/// Loads the kernel's own GDT, reloads every segment register from it, and
/// points the task register at the main TSS.
///
/// [`crate::tss::init`] must have run first: this builds descriptors for both
/// TSSes, and `ltr` makes the CPU read the main one.
pub fn init() {
    // SAFETY: single-threaded, interrupts masked, and no task switch is
    // possible yet - the task register is loaded at the end of this function.
    let gdt = unsafe { &mut *GDT.0.get() };

    // Written out at runtime, not left to the static's initialiser: the TSS
    // descriptors depend on addresses only known at runtime, and `.bss`/
    // `.rodata` contents wouldn't survive the flat-binary load anyway (see
    // `idt::init`).
    gdt.entries[NULL_INDEX] = Descriptor::NULL;
    gdt.entries[KERNEL_CODE_INDEX] = Descriptor::flat(ACCESS_KERNEL_CODE);
    gdt.entries[KERNEL_DATA_INDEX] = Descriptor::flat(ACCESS_KERNEL_DATA);
    gdt.entries[MAIN_TSS_INDEX] = Descriptor::tss(&tss::main_task());
    gdt.entries[DOUBLE_FAULT_TSS_INDEX] = Descriptor::tss(&tss::double_fault_task());

    let pointer = GdtPointer {
        limit: (size_of::<Gdt>() - 1) as u16,
        base: gdt as *const Gdt as u32,
    };
    // SAFETY: `pointer` describes `GDT`, whose entries were just written:
    // entry 1 is a flat ring-0 code segment and entry 2 a flat ring-0
    // writable data segment, as `load` requires.
    unsafe { load(&pointer) };
    // SAFETY: entry 3 is an available 32-bit TSS descriptor for the
    // `'static` main TSS, which `tss::init` has fully initialised.
    unsafe { load_task_register(MAIN_TSS_SELECTOR) };
}

/// # Safety
/// `pointer` must describe a valid, 8-byte-aligned GDT that stays resident
/// for as long as the CPU uses it, whose entry 1 is a ring-0 32-bit code
/// segment and entry 2 a ring-0 writable data segment - both flat (base 0,
/// 4 GiB limit), since the currently-executing code, its stack, and all its
/// data pointers are physical addresses that must keep resolving unchanged
/// across the switch.
unsafe fn load(pointer: &GdtPointer) {
    unsafe {
        asm!(
            // CS can only be reloaded by a far transfer, so jump to the very
            // next instruction through the new code selector. The remaining
            // segment registers - including SS, which is why the stack must
            // stay flat - are plain assignments.
            "lgdt ({gdtr})",
            "ljmp $0x08, $2f",
            "2:",
            "movw $0x10, %ax",
            "movw %ax, %ds",
            "movw %ax, %es",
            "movw %ax, %fs",
            "movw %ax, %gs",
            "movw %ax, %ss",
            gdtr = in(reg) pointer,
            out("eax") _,
            options(att_syntax, preserves_flags),
        );
    }
}

/// # Safety
/// `selector` must name a GDT entry holding an *available* (not already busy)
/// 32-bit TSS descriptor whose TSS stays resident for as long as the CPU can
/// take a task switch. The CPU marks that descriptor busy and will save the
/// current register set into the TSS on the next task gate, so a bogus
/// selector turns the double-fault handler - the last line of defence - into a
/// triple fault.
unsafe fn load_task_register(selector: u16) {
    unsafe {
        asm!(
            "ltr {selector:x}",
            selector = in(reg) selector,
            options(att_syntax, nostack, preserves_flags),
        );
    }
}

/// Reads back the GDT the CPU is currently using, the code/data selectors
/// currently in use, and the task register.
pub fn loaded() -> Loaded {
    let mut pointer = GdtPointer::default();
    let cs: u16;
    let ds: u16;
    let tr: u16;
    unsafe {
        asm!(
            "sgdt ({gdtr})",
            "movw %cs, {cs:x}",
            "movw %ds, {ds:x}",
            "str {tr:x}",
            gdtr = in(reg) &mut pointer,
            cs = out(reg) cs,
            ds = out(reg) ds,
            tr = out(reg) tr,
            options(att_syntax, nostack, preserves_flags),
        );
    }
    Loaded {
        base: pointer.base,
        limit: pointer.limit,
        cs,
        ds,
        tr,
    }
}
