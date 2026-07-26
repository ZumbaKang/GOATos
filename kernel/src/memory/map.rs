//! The BIOS memory map, as handed over by the bootloader.
//!
//! `boot/boot.asm` walks `INT 15h, EAX=E820h` while still in real mode and
//! writes the raw entries to a fixed low-memory address ([`HANDOFF_ADDR`]),
//! because that call is unavailable the moment the CPU enters protected mode.
//! This module is the other end of that agreement: it validates the block,
//! copies the entries out of the scratch area, and presents them as
//! [`Region`]s.
//!
//! The block's layout, all little-endian:
//!
//! ```text
//! +0   u32  signature ("GOAT"), written before the walk starts
//! +4   u32  number of entries stored
//! +8        entries, 24 bytes each: u64 base, u64 length, u32 type,
//!           u32 ACPI 3.0 extended attributes
//! ```
//!
//! Defensive by design, like the other early-boot code here: a BIOS with no
//! E820 support, a bootloader that never got as far as writing the block, and
//! a map longer than there is room for all have to end in a report rather than
//! a wrong answer or a hang, since a bogus memory map would otherwise be
//! discovered much later as an inexplicable fault in some allocator.

use core::fmt;
use core::ptr;

/// Physical address of the handoff block. Must match `MEMMAP_ADDR` in
/// `boot/boot.asm`.
pub const HANDOFF_ADDR: usize = 0x500;

/// Marks the block as one the bootloader actually wrote. Low memory is not
/// guaranteed to be zero (only an emulator's fresh RAM is), so "the count
/// looks plausible" is not on its own evidence that anything was handed over.
const SIGNATURE: u32 = 0x5441_4f47; // "GOAT", little-endian

/// How many entries the bootloader has room for; must match
/// `MEMMAP_MAX_ENTRIES` in `boot/boot.asm`. Real machines report well under a
/// dozen regions, so this is generous, but a longer map is truncated rather
/// than silently wrapped - see [`MemoryMap::truncated`].
pub const MAX_ENTRIES: usize = 32;

/// Byte offset of the first entry within the block.
const ENTRIES_OFFSET: usize = 8;

/// Size of one raw E820 entry: the 20 bytes every BIOS returns, plus the ACPI
/// 3.0 extended attributes dword.
const ENTRY_SIZE: usize = 24;

// Every field of every entry is naturally aligned only because both the block
// itself and the entry stride are multiples of 8. Keep it that way, or the
// reads below need `read_unaligned`.
const _: () = assert!(HANDOFF_ADDR.is_multiple_of(8));
const _: () = assert!(ENTRIES_OFFSET.is_multiple_of(8) && ENTRY_SIZE.is_multiple_of(8));

/// What the BIOS says a region is for. The numbering is E820's; anything
/// outside it must be treated as unusable, which is why [`RegionKind::Unknown`]
/// exists instead of a fallback to "reserved".
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RegionKind {
    /// Ordinary RAM, free for the kernel to use.
    Usable,
    /// Real memory the firmware or hardware has claimed, or a hole with no RAM
    /// behind it at all.
    Reserved,
    /// Holds ACPI tables; usable once the kernel has read (or decided to
    /// ignore) them. Treated as reserved until then.
    AcpiReclaimable,
    /// ACPI non-volatile storage: must be preserved across sleep states, so
    /// never usable.
    AcpiNvs,
    /// RAM the firmware knows to be faulty.
    BadMemory,
    /// A type this kernel does not know. Not usable, by definition: the safe
    /// reading of "I don't recognise this" is "don't touch it".
    Unknown(u32),
}

impl RegionKind {
    fn from_raw(raw: u32) -> RegionKind {
        match raw {
            1 => RegionKind::Usable,
            2 => RegionKind::Reserved,
            3 => RegionKind::AcpiReclaimable,
            4 => RegionKind::AcpiNvs,
            5 => RegionKind::BadMemory,
            other => RegionKind::Unknown(other),
        }
    }
}

impl fmt::Display for RegionKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RegionKind::Usable => f.write_str("usable"),
            RegionKind::Reserved => f.write_str("reserved"),
            RegionKind::AcpiReclaimable => f.write_str("ACPI reclaimable"),
            RegionKind::AcpiNvs => f.write_str("ACPI NVS"),
            RegionKind::BadMemory => f.write_str("bad memory"),
            RegionKind::Unknown(raw) => write!(f, "unknown (type {})", raw),
        }
    }
}

/// One contiguous physical address range, as the BIOS described it.
///
/// Addresses are 64-bit even though this kernel is 32-bit: E820 reports the
/// machine's whole physical address space, and a PC routinely has regions
/// above 4GiB (the PCI hole is remapped up there). They are kept as reported
/// rather than clamped, so that whoever consumes them later decides what to do
/// with what this CPU cannot reach.
#[derive(Clone, Copy)]
pub struct Region {
    /// First byte of the region.
    pub base: u64,
    /// Size in bytes. Never zero: empty regions are dropped on the way in.
    pub length: u64,
    /// What the BIOS says the region is.
    pub kind: RegionKind,
}

impl Region {
    /// One past the last byte of the region. Saturating, so a BIOS reporting a
    /// region that runs off the end of the address space cannot wrap it to
    /// something that looks like a low address.
    pub fn end(&self) -> u64 {
        self.base.saturating_add(self.length)
    }

    /// Whether the kernel is free to use this memory.
    pub fn is_usable(&self) -> bool {
        self.kind == RegionKind::Usable
    }
}

/// The machine's physical memory layout, as [`load`] found it.
pub struct MemoryMap {
    regions: [Region; MAX_ENTRIES],
    len: usize,
    /// How many entries the bootloader said it stored, which can exceed
    /// `len` - see [`MemoryMap::truncated`].
    reported: usize,
    available: bool,
}

impl MemoryMap {
    const fn empty() -> MemoryMap {
        MemoryMap {
            regions: [Region {
                base: 0,
                length: 0,
                kind: RegionKind::Reserved,
            }; MAX_ENTRIES],
            len: 0,
            reported: 0,
            available: false,
        }
    }

    /// Whether the bootloader handed over a map at all. False means the
    /// signature was missing - the BIOS has no E820 service, or the handoff
    /// block was never written - and every other accessor here reports an
    /// empty map.
    pub fn available(&self) -> bool {
        self.available
    }

    /// Whether the BIOS reported more regions than the handoff block can hold,
    /// so the tail of the map is missing. The regions that *are* here are
    /// still accurate; there are just more of them somewhere.
    pub fn truncated(&self) -> bool {
        self.reported > MAX_ENTRIES
    }

    /// The regions, in the order the BIOS reported them (which is ascending by
    /// base address on every BIOS worth the name, but is not guaranteed).
    pub fn regions(&self) -> &[Region] {
        &self.regions[..self.len]
    }

    /// Total bytes of [`RegionKind::Usable`] memory.
    pub fn total_usable(&self) -> u64 {
        self.regions()
            .iter()
            .filter(|region| region.is_usable())
            .fold(0u64, |total, region| total.saturating_add(region.length))
    }

    /// Highest address the BIOS mentioned at all, usable or not - the top of
    /// the physical address space as far as this machine is concerned.
    pub fn highest_address(&self) -> u64 {
        self.regions()
            .iter()
            .map(Region::end)
            .max()
            .unwrap_or_default()
    }
}

impl fmt::Display for MemoryMap {
    /// A one-line summary, for the boot banner. The unavailable case is
    /// deliberately loud: every later piece of memory management is built on
    /// this map, so silently reporting "0 regions" would turn a missing
    /// handoff into a much more confusing failure further along.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if !self.available() {
            return f.write_str(
                "Memory: MEMORY MAP UNAVAILABLE (no E820 handoff from the boot sector)",
            );
        }
        write!(
            f,
            "Memory: {} E820 regions, {} MiB usable, top {:#012x}",
            self.regions().len(),
            self.total_usable() / (1024 * 1024),
            self.highest_address()
        )?;
        if self.truncated() {
            write!(f, " (TRUNCATED at {} of {})", MAX_ENTRIES, self.reported)?;
        }
        Ok(())
    }
}

/// Reads the handoff block the bootloader left behind.
///
/// Never fails: an absent or unreadable map comes back as an empty
/// [`MemoryMap`] with [`MemoryMap::available`] false, for the caller to report.
/// Entries the BIOS marked as ignorable, and zero-length ones, are dropped
/// here rather than being pushed onto every consumer.
pub fn load() -> MemoryMap {
    let mut map = MemoryMap::empty();

    // SAFETY: the whole block sits in the low-memory scratch area that
    // `boot/boot.asm` reserves for it, well below the kernel's own load
    // address, and paging is off, so these are plain physical addresses that
    // nothing else in the kernel writes. `read_volatile` because the values
    // were written by code the compiler cannot see (the bootloader, and the
    // BIOS underneath it). Alignment is guaranteed by the const asserts above.
    let signature = unsafe { ptr::read_volatile(HANDOFF_ADDR as *const u32) };
    if signature != SIGNATURE {
        return map;
    }
    map.available = true;

    // SAFETY: as above; the count is only meaningful once the signature has
    // been matched, which is why it is read second.
    map.reported = unsafe { ptr::read_volatile((HANDOFF_ADDR + 4) as *const u32) } as usize;

    for index in 0..map.reported.min(MAX_ENTRIES) {
        let entry = (HANDOFF_ADDR + ENTRIES_OFFSET + index * ENTRY_SIZE) as *const u8;
        // SAFETY: as above, and `index` is bounded by the number of entries
        // the block has room for, so this stays inside the reserved area.
        let (base, length, kind, attributes) = unsafe {
            (
                ptr::read_volatile(entry as *const u64),
                ptr::read_volatile(entry.add(8) as *const u64),
                ptr::read_volatile(entry.add(16) as *const u32),
                ptr::read_volatile(entry.add(20) as *const u32),
            )
        };

        // ACPI 3.0 added an attributes dword whose bit 0 clear means "ignore
        // this entry entirely". The bootloader pre-sets it, so a BIOS that
        // only writes the older 20-byte entry still reads as valid here.
        if length == 0 || attributes & 1 == 0 {
            continue;
        }

        map.regions[map.len] = Region {
            base,
            length,
            kind: RegionKind::from_raw(kind),
        };
        map.len += 1;
    }

    map
}
