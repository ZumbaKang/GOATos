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
//! One deliberate hole: the 4 KiB page immediately below the kernel stack
//! (`stack_guard_page` in `entry.s`) is left not-present. An overflow that
//! grows `esp` into that page cannot push an interrupt frame, so the resulting
//! page fault escalates to a double fault on the private stack from task 1.4.
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
    /// Start of the deliberately unmapped stack guard page, or 0 if setup
    /// failed before one could be reserved.
    stack_guard: u32,
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

    /// Start address of the unmapped stack guard page.
    pub fn stack_guard(self) -> u32 {
        self.stack_guard
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
        )?;
        if self.stack_guard != 0 {
            write!(
                f,
                "\nPaging: stack guard page {:#010x}-{:#010x} unmapped",
                self.stack_guard,
                self.stack_guard + FRAME_SIZE
            )?;
        }
        Ok(())
    }
}

extern "C" {
    static stack_guard_page: u8;
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
            stack_guard: 0,
        };
    }

    // The page immediately below the kernel stack - left not-present so an
    // overflow faults instead of corrupting the double-fault stack that sits
    // under it. Address only; the bytes are never touched.
    let stack_guard = core::ptr::addr_of!(stack_guard_page) as u32;
    debug_assert!(stack_guard.is_multiple_of(FRAME_SIZE));

    let page_table_count = (mapped_end / BYTES_PER_DIRECTORY_ENTRY) as usize;

    let Some(directory) = allocate_zeroed_frame() else {
        return Report {
            cr3: 0,
            mapped_end: 0,
            page_tables: 0,
            paging_enabled: false,
            stack_guard: 0,
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
                stack_guard: 0,
            };
        };

        let base = (table_index as u32) * BYTES_PER_DIRECTORY_ENTRY;
        for page_index in 0..ENTRIES {
            let phys = base + (page_index as u32) * FRAME_SIZE;
            // Deliberate hole: the stack guard page stays not-present.
            if phys == stack_guard {
                continue;
            }
            write_entry(table, page_index, phys | KERNEL_PAGE);
        }
        write_entry(directory, table_index, table.start_address() | KERNEL_PAGE);
    }

    let cr3 = directory.start_address();

    // A task switch loads CR3 from the incoming TSS. Both TSSes must already
    // carry the real directory before PG goes high, or a double fault taken
    // with paging on would switch onto a directory of zeroes and triple-fault.
    tss::set_page_directory(cr3);

    // SAFETY: `cr3` points at a zeroed page directory whose identity map
    // covers every address this kernel currently executes, reads, or writes -
    // including the directory and tables themselves, the kernel image, both
    // stacks, and the VGA text buffer - with the single exception of the
    // stack guard page, which nothing is supposed to touch. Enabling paging
    // under any thinner map would page-fault on the next instruction.
    unsafe {
        enable(cr3);
    }

    let enabled = paging_is_enabled();
    let active_cr3 = read_cr3();
    // Confirm the hole stuck: a present PTE here would mean the skip above
    // never ran, and stack overflow would corrupt memory again.
    let guard_unmapped = !is_present(stack_guard);

    Report {
        // Prefer the value the CPU accepted over the one we wrote, so a
        // mismatch (masked bits, wrong load) shows up in the banner.
        cr3: active_cr3,
        mapped_end,
        page_tables: page_table_count,
        paging_enabled: enabled && active_cr3 == cr3 && guard_unmapped,
        stack_guard: if guard_unmapped { stack_guard } else { 0 },
    }
}

/// Whether the page containing `virt` has its present bit set in the active
/// page tables. Used by the layout check to prove the stack guard stayed
/// unmapped after [`init`].
pub fn is_present(virt: u32) -> bool {
    let cr3 = read_cr3();
    if cr3 == 0 {
        return false;
    }
    let pd_index = (virt / BYTES_PER_DIRECTORY_ENTRY) as usize;
    let pt_index = ((virt % BYTES_PER_DIRECTORY_ENTRY) / FRAME_SIZE) as usize;
    // SAFETY: `cr3` is the directory we installed (or zero, handled above).
    // Reading a PDE/PTE is a plain load through the identity map; directory
    // and tables are themselves mapped.
    let pde = unsafe { read_entry(Frame::containing_address(cr3), pd_index) };
    if pde & PRESENT == 0 {
        return false;
    }
    let table = Frame::containing_address(pde & !(FRAME_SIZE - 1));
    let pte = unsafe { read_entry(table, pt_index) };
    pte & PRESENT != 0
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

/// # Safety
///
/// `table` must be a mapped page-directory or page-table frame, and `index`
/// must be in `0..ENTRIES`.
unsafe fn read_entry(table: Frame, index: usize) -> u32 {
    debug_assert!(index < ENTRIES);
    let ptr = (table.start_address() as *const u32).add(index);
    ptr::read_volatile(ptr)
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
