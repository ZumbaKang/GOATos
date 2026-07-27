//! 32-bit non-PAE paging: a page directory, page tables, and identity maps.
//!
//! Until this module runs, every address the kernel uses is a physical one -
//! the bootloader left paging off. Turning it on is a one-way door: the
//! moment `CR0.PG` is set the CPU starts translating every fetch and every
//! load through the tables pointed at by `CR3`, and anything not present in
//! those tables faults. So the tables have to cover *everything* the kernel
//! is already using (its own image, both stacks, the VGA buffer, the frame
//! allocator's pool) *before* that bit flips.
//!
//! Identity mapping - virtual address == physical address - is the simplest
//! correct starting point. A higher-half kernel can come later; for now the
//! translation is a no-op for every address that matters, which is exactly
//! what lets the existing code keep running after paging is enabled.
//!
//! Layout, 32-bit non-PAE (two levels, 4 KiB pages):
//!
//! ```text
//! CR3 -> page directory (1024 x 4-byte entries, one per 4 MiB)
//!          -> page table  (1024 x 4-byte entries, one per 4 KiB)
//!               -> physical frame
//! ```
//!
//! Each directory and each table is itself one 4 KiB frame, taken from
//! [`super::frame`]. Frames come back un-zeroed, so every table is wiped
//! before any entry is written - a leftover free-list link in the first
//! four bytes would otherwise look like a present mapping at a nonsense
//! address.

use core::arch::asm;
use core::fmt;
use core::ptr;

use super::frame::{self, Frame, FRAME_SIZE};
use super::map::MemoryMap;
use crate::tss;

/// Present bit: the translation is valid. Without it the CPU raises #PF.
const PRESENT: u32 = 1 << 0;
/// Read/write bit: the page is writable. Kernel code and the VGA buffer both
/// need this; a read-only identity map would fault the first `vga_print!`.
const WRITABLE: u32 = 1 << 1;

/// Flags every identity-mapped kernel page gets. User-mode access is left
/// off: there is no ring 3 yet, and nothing here should be reachable from one
/// later without an explicit decision.
const KERNEL_PAGE: u32 = PRESENT | WRITABLE;

/// Entries in a page directory or a page table. Fixed by the architecture.
const ENTRIES: usize = 1024;
const _: () = assert!(ENTRIES * 4 == FRAME_SIZE as usize);

/// Bytes one page-directory entry covers (one whole page table of 4 KiB pages).
const BYTES_PER_DIRECTORY_ENTRY: u32 = FRAME_SIZE * ENTRIES as u32;
const _: () = assert!(BYTES_PER_DIRECTORY_ENTRY == 4 * 1024 * 1024);

/// `CR0` bit 31 - paging enable. The other bits (including PE) are already
/// set by the bootloader and must be preserved.
const CR0_PG: u32 = 1 << 31;

/// What [`init`] built and whether it stuck. Printed on the boot banner so a
/// headless log can prove paging is on without attaching a debugger.
#[derive(Clone, Copy)]
pub struct Report {
    /// Physical address of the page directory, also the value in `CR3`.
    cr3: u32,
    /// One past the last identity-mapped byte. Always a multiple of 4 MiB.
    mapped_end: u32,
    /// Page tables hung off the directory (one per 4 MiB of mapped space).
    page_tables: usize,
    /// Whether `CR0.PG` reads back as set after the enable sequence.
    paging_enabled: bool,
}

impl Report {
    /// Physical address of the active page directory (`CR3`).
    pub fn cr3(self) -> u32 {
        self.cr3
    }

    /// Exclusive end of the identity-mapped window.
    pub fn mapped_end(self) -> u32 {
        self.mapped_end
    }

    /// How many 4 MiB page tables were installed.
    pub fn page_tables(self) -> usize {
        self.page_tables
    }

    /// Whether paging is actually on, per `CR0`.
    pub fn paging_enabled(self) -> bool {
        self.paging_enabled
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.cr3 == 0 {
            return f.write_str("Paging: FAILED (could not build an identity map)");
        }
        let pg = u8::from(self.paging_enabled);
        write!(
            f,
            "Paging: identity-mapped {:#010x}-{:#010x} ({} MiB) via {} page tables, CR3={:#010x}, PG={}",
            0u32,
            self.mapped_end,
            self.mapped_end / (1024 * 1024),
            self.page_tables,
            self.cr3,
            pg
        )
    }
}

/// Builds an identity map covering every usable frame below 4 GiB (plus the
/// holes between them, so the VGA buffer at 0xb8000 stays reachable), loads
/// it into `CR3`, updates both TSSes so a double-fault task switch keeps the
/// same directory, and enables paging.
///
/// Returns a report either way: a machine with no usable memory, or one that
/// runs out of frames mid-setup, gets a loud failure line rather than a
/// silent halt with `CR0.PG` half-applied.
pub fn init(map: &MemoryMap) -> Report {
    let mapped_end = identity_map_end(map);
    if mapped_end == 0 {
        return Report {
            cr3: 0,
            mapped_end: 0,
            page_tables: 0,
            paging_enabled: false,
        };
    }

    let page_table_count = (mapped_end / BYTES_PER_DIRECTORY_ENTRY) as usize;

    let Some(directory) = allocate_zeroed_frame() else {
        return Report {
            cr3: 0,
            mapped_end: 0,
            page_tables: 0,
            paging_enabled: false,
        };
    };

    for table_index in 0..page_table_count {
        let Some(table) = allocate_zeroed_frame() else {
            // Leave whatever we already built allocated: abandoning paging
            // mid-setup is a boot failure either way, and freeing a partial
            // tree is more code than the failure path is worth.
            return Report {
                cr3: 0,
                mapped_end: 0,
                page_tables: 0,
                paging_enabled: false,
            };
        };

        let base = (table_index as u32) * BYTES_PER_DIRECTORY_ENTRY;
        for page_index in 0..ENTRIES {
            let phys = base + (page_index as u32) * FRAME_SIZE;
            write_entry(table, page_index, phys | KERNEL_PAGE);
        }
        write_entry(directory, table_index, table.start_address() | KERNEL_PAGE);
    }

    let cr3 = directory.start_address();

    // A task switch loads CR3 from the incoming TSS. Both TSSes must already
    // carry the real directory before PG goes high, or a double fault taken
    // with paging on would switch onto a directory of zeroes and triple-fault.
    tss::set_page_directory(cr3);

    // SAFETY: `cr3` points at a zeroed, fully-populated page directory whose
    // identity map covers every address this kernel currently executes,
    // reads, or writes - including the directory and tables themselves, the
    // kernel image, both stacks, and the VGA text buffer. Enabling paging
    // under any thinner map would page-fault on the next instruction.
    unsafe {
        enable(cr3);
    }

    let enabled = paging_is_enabled();
    let active_cr3 = read_cr3();

    Report {
        // Prefer the value the CPU accepted over the one we wrote, so a
        // mismatch (masked bits, wrong load) shows up in the banner.
        cr3: active_cr3,
        mapped_end,
        page_tables: page_table_count,
        paging_enabled: enabled && active_cr3 == cr3,
    }
}

/// Exclusive end of the window [`init`] identity-maps: the highest usable
/// byte below 4 GiB, rounded up to the next 4 MiB boundary so it lands on a
/// page-directory entry boundary.
///
/// Rounding *up* (and mapping the holes under that ceiling, not just the
/// usable regions) is deliberate: the VGA text buffer at 0xb8000 sits in the
/// legacy hole below 1 MiB, which no E820 usable region covers, and the
/// kernel would fault the first time it printed after enabling paging if
/// that hole were left unmapped.
fn identity_map_end(map: &MemoryMap) -> u32 {
    let mut end: u64 = 0;
    for region in map.regions().iter().filter(|region| region.is_usable()) {
        end = end.max(region.end().min(1u64 << 32));
    }
    if end == 0 {
        return 0;
    }
    let rounded = end.div_ceil(BYTES_PER_DIRECTORY_ENTRY as u64) * BYTES_PER_DIRECTORY_ENTRY as u64;
    // A machine that really reports 4 GiB of usable RAM would round to a
    // value that does not fit in `u32`. Cap at the last full 4 MiB below the
    // 4 GiB line rather than wrapping; QEMU's 128 MiB default never gets
    // near this.
    rounded.min((1u64 << 32) - BYTES_PER_DIRECTORY_ENTRY as u64) as u32
}

fn allocate_zeroed_frame() -> Option<Frame> {
    let frame = frame::allocate()?;
    // SAFETY: the frame allocator just handed this frame over, so nothing
    // else holds it. Page tables must start zeroed: a free-list link left in
    // the first word would decode as a present PDE/PTE at a garbage address.
    unsafe {
        let ptr = frame.start_address() as *mut u8;
        ptr::write_bytes(ptr, 0, FRAME_SIZE as usize);
    }
    Some(frame)
}

fn write_entry(table: Frame, index: usize, value: u32) {
    debug_assert!(index < ENTRIES);
    // SAFETY: `table` is a frame we own and zeroed; `index` is in range for a
    // 1024-entry directory/table. `write_volatile` so the CPU sees the stores
    // before `mov %cr3`, even if nothing reads the entries back from Rust.
    unsafe {
        let ptr = (table.start_address() as *mut u32).add(index);
        ptr::write_volatile(ptr, value);
    }
}

/// Loads `CR3` and sets `CR0.PG`.
///
/// # Safety
///
/// `page_directory` must be a 4 KiB-aligned physical address of a valid page
/// directory that identity-maps every virtual address the kernel will touch
/// between this call returning and any later remapping - at minimum the
/// current instruction stream, stack, and data the next Rust statement uses.
unsafe fn enable(page_directory: u32) {
    asm!(
        "mov {pd}, %cr3",
        "mov %cr0, %eax",
        "orl ${pg}, %eax",
        "mov %eax, %cr0",
        pd = in(reg) page_directory,
        pg = const CR0_PG,
        out("eax") _,
        options(att_syntax, nostack),
    );
}

fn read_cr3() -> u32 {
    let cr3: u32;
    // SAFETY: reading CR3 has no side effects.
    unsafe {
        asm!("mov %cr3, {}", out(reg) cr3, options(att_syntax, nomem, nostack, preserves_flags));
    }
    cr3
}

fn read_cr0() -> u32 {
    let cr0: u32;
    // SAFETY: reading CR0 has no side effects.
    unsafe {
        asm!("mov %cr0, {}", out(reg) cr0, options(att_syntax, nomem, nostack, preserves_flags));
    }
    cr0
}

fn paging_is_enabled() -> bool {
    read_cr0() & CR0_PG != 0
}
