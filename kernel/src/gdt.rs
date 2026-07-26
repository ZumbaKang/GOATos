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

use core::arch::asm;
use core::mem::size_of;

/// Access byte for a ring-0, present, executable + readable code segment.
const ACCESS_KERNEL_CODE: u8 = 0b1001_1010;
/// Access byte for a ring-0, present, writable data segment.
const ACCESS_KERNEL_DATA: u8 = 0b1001_0010;
/// Flags nibble: 4 KiB granularity + 32-bit segment.
const FLAGS_32BIT_4K: u8 = 0b1100;

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
}

/// The table itself. 8-byte aligned because the CPU reads entries as 8-byte
/// quantities, and a `static` so its address stays valid for the whole life
/// of the kernel (the CPU keeps reading it out of memory long after `init`).
#[repr(C, align(8))]
struct Gdt {
    entries: [Descriptor; 3],
}

const NULL_INDEX: usize = 0;
const KERNEL_CODE_INDEX: usize = 1;
const KERNEL_DATA_INDEX: usize = 2;

static GDT: Gdt = Gdt {
    entries: {
        let mut entries = [Descriptor::NULL; 3];
        entries[NULL_INDEX] = Descriptor::NULL;
        entries[KERNEL_CODE_INDEX] = Descriptor::flat(ACCESS_KERNEL_CODE);
        entries[KERNEL_DATA_INDEX] = Descriptor::flat(ACCESS_KERNEL_DATA);
        entries
    },
};

/// Selector (byte offset into the GDT) for the kernel code segment.
pub const KERNEL_CODE_SELECTOR: u16 = (KERNEL_CODE_INDEX * 8) as u16;
/// Selector (byte offset into the GDT) for the kernel data segment.
pub const KERNEL_DATA_SELECTOR: u16 = (KERNEL_DATA_INDEX * 8) as u16;

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

/// What the CPU currently has loaded, read back with `sgdt` plus the segment
/// registers - enough to prove which GDT is actually in effect.
pub struct Loaded {
    pub base: u32,
    pub limit: u16,
    pub cs: u16,
    pub ds: u16,
}

/// Loads the kernel's own GDT and reloads every segment register from it,
/// replacing the bootloader's temporary table.
pub fn init() {
    let pointer = GdtPointer {
        limit: (size_of::<Gdt>() - 1) as u16,
        base: &GDT as *const Gdt as u32,
    };
    unsafe { load(&pointer) };
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

/// Reads back the GDT the CPU is currently using, and the code/data selectors
/// currently in use.
pub fn loaded() -> Loaded {
    let mut pointer = GdtPointer::default();
    let cs: u16;
    let ds: u16;
    unsafe {
        asm!(
            "sgdt ({gdtr})",
            "movw %cs, {cs:x}",
            "movw %ds, {ds:x}",
            gdtr = in(reg) &mut pointer,
            cs = out(reg) cs,
            ds = out(reg) ds,
            options(att_syntax, nostack, preserves_flags),
        );
    }
    Loaded {
        base: pointer.base,
        limit: pointer.limit,
        cs,
        ds,
    }
}
