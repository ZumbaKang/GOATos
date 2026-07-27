//! Physical frame allocator: who owns each 4 KiB page of RAM.
//!
//! [`super::map`] says which physical addresses are real, usable RAM. This
//! module is the first thing to act on that: it turns those regions into a
//! pool of fixed-size *frames* and hands them out one at a time. Paging
//! (a page directory and page tables are themselves 4 KiB, frame-aligned
//! structures) and the kernel heap both start here.
//!
//! Two things make "usable RAM" narrower than the BIOS reports, and getting
//! either wrong is the kind of bug that shows up much later as memory
//! corruption in something unrelated:
//!
//! - Some of that RAM is *already occupied* - by the kernel image itself, by
//!   the E820 handoff block, by the boot sector. The BIOS has no way to know
//!   that; it reports the machine, not what this kernel did to it. So the
//!   allocator carves out an explicit list of [`Reservation`]s, and every
//!   frame it hands out is checked against them.
//! - E820 addresses are 64-bit, and a PC really does report regions above
//!   4 GiB. This CPU cannot address them with paging off (or, later, with
//!   32-bit non-PAE paging on), so anything past the 4 GiB line is dropped.
//!
//! The allocator itself is a bump pointer over the surviving regions plus a
//! free list for anything handed back. The free list is *intrusive*: a free
//! frame stores the index of the next free frame in its own first four bytes,
//! which is the standard trick for having a free list before there is a heap
//! to keep one in.

use core::cmp::Ordering;
use core::fmt;
use core::ptr;
use spin::Mutex;

use super::map::MemoryMap;

/// Size of a physical frame, which is also the page size 32-bit x86 paging
/// uses (the 4 MiB "large page" alternative needs a CPU feature bit and buys
/// nothing yet).
pub const FRAME_SIZE: u32 = 4096;

/// `log2(FRAME_SIZE)`: an address's frame index is its top 20 bits.
const FRAME_SHIFT: u32 = 12;
const _: () = assert!(FRAME_SIZE == 1 << FRAME_SHIFT);

/// One past the last frame index a 32-bit physical address can name. Every
/// index this module stores is below it, which is what makes
/// [`Frame::start_address`] infallible.
const MAX_FRAME_INDEX: u32 = 1 << (32 - FRAME_SHIFT);

/// Sentinel for "no frame" in the intrusive free list. `MAX_FRAME_INDEX` and
/// everything above it is not a valid index, so this cannot collide with a
/// real one.
const NO_FRAME: u32 = u32::MAX;

/// How many usable regions the allocator can track. The memory map cannot
/// hand over more than it can hold, so this can never be the binding limit.
const MAX_REGIONS: usize = super::map::MAX_ENTRIES;

/// How many ranges [`FrameAllocator::new`] excludes. Fixed and small: these
/// are the parts of physical memory this kernel has already put something in,
/// and they are all known statically except for the kernel's own extent.
const MAX_RESERVATIONS: usize = 4;

/// End of the low-memory area the BIOS and the boot sector's handoff block
/// share: the real-mode interrupt vector table (0x0), the BIOS data area
/// (0x400), and the E820 block at [`super::map::HANDOFF_ADDR`], which is read
/// back well after this allocator starts running.
const BIOS_LOW_MEMORY_END: u32 = 0x1000;

/// The page the boot sector was loaded into (0x7c00), which is also where its
/// real-mode stack grew down from.
const BOOT_SECTOR_PAGE: u32 = 0x7000;

/// The legacy video/option-ROM/BIOS window just below 1 MiB - the VGA text
/// buffer at 0xb8000 is in here. A sane BIOS reports all of this as unusable,
/// but "the firmware said it was fine to overwrite the screen" is not a claim
/// worth taking on trust.
const LEGACY_ROM_START: u32 = 0xa_0000;
const LEGACY_ROM_END: u32 = 0x10_0000;

/// One physical 4 KiB frame, named by its index rather than its address so
/// that the frame just below the 4 GiB line is representable without
/// overflowing.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Frame {
    index: u32,
}

impl Frame {
    const fn from_index(index: u32) -> Frame {
        Frame { index }
    }

    /// The frame this physical address falls in.
    pub const fn containing_address(address: u32) -> Frame {
        Frame {
            index: address >> FRAME_SHIFT,
        }
    }

    /// Index of the frame, i.e. its address divided by [`FRAME_SIZE`].
    pub const fn index(self) -> u32 {
        self.index
    }

    /// Physical address of the frame's first byte, always [`FRAME_SIZE`]-aligned.
    pub const fn start_address(self) -> u32 {
        self.index << FRAME_SHIFT
    }
}

impl fmt::Display for Frame {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:#010x}", self.start_address())
    }
}

/// A half-open run of frames, `start..end` in frame indices.
#[derive(Clone, Copy)]
struct FrameRange {
    start: u32,
    end: u32,
}

impl FrameRange {
    const EMPTY: FrameRange = FrameRange { start: 0, end: 0 };

    /// The frames lying entirely *inside* a byte range - used for usable RAM,
    /// where a partial frame at either end is not something to hand out.
    fn inside(start: u64, end: u64) -> FrameRange {
        let (start, end) = (index_ceil(start), index_floor(end));
        FrameRange {
            start,
            end: end.max(start),
        }
    }

    /// The frames a byte range touches at all - used for reservations, where
    /// a single occupied byte has to take the whole frame out of circulation.
    fn covering(start: u32, end: u32) -> FrameRange {
        FrameRange {
            start: index_floor(start as u64),
            end: index_ceil(end as u64),
        }
    }

    fn contains(&self, index: u32) -> bool {
        index >= self.start && index < self.end
    }

    fn len(&self) -> usize {
        (self.end - self.start) as usize
    }

    fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

/// Index of the frame containing `address`, saturating at the top of the
/// 32-bit address space so a 64-bit E820 address cannot wrap into a low one.
fn index_floor(address: u64) -> u32 {
    (address >> FRAME_SHIFT).min(MAX_FRAME_INDEX as u64) as u32
}

/// As [`index_floor`], rounded the other way.
fn index_ceil(address: u64) -> u32 {
    index_floor(address.saturating_add(FRAME_SIZE as u64 - 1))
}

/// A range of physical memory the allocator must never hand out, and why.
///
/// The reason is carried around because it is the useful half at boot: a
/// range of addresses says nothing about whether excluding it was correct,
/// but "kernel image, .bss and stack" printed next to the kernel's actual
/// extent can be checked against the linker's own idea of it.
#[derive(Clone, Copy)]
pub struct Reservation {
    range: FrameRange,
    reason: &'static str,
}

impl Reservation {
    /// Address of the first reserved byte.
    pub fn start_address(&self) -> u64 {
        (self.range.start as u64) << FRAME_SHIFT
    }

    /// One past the last reserved byte. 64-bit because a reservation may run
    /// all the way to the 4 GiB line.
    pub fn end_address(&self) -> u64 {
        (self.range.end as u64) << FRAME_SHIFT
    }

    /// How many frames the reservation takes out of circulation.
    pub fn frames(&self) -> usize {
        self.range.len()
    }
}

impl fmt::Display for Reservation {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{:#010x}-{:#010x} {:>5} frames  {}",
            self.start_address(),
            self.end_address(),
            self.frames(),
            self.reason
        )
    }
}

/// Why a frame could not be given back. All three mean the *caller* is
/// confused rather than the allocator, which is exactly why they are reported
/// instead of ignored: a frame freed twice would be handed to two owners at
/// once, which is far harder to debug than an error at the point of the
/// mistake.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FreeError {
    /// The frame is not part of the pool - outside every usable region, or
    /// inside a reservation.
    NotManaged,
    /// A frame in the pool, but one the allocator has never handed out.
    NeverAllocated,
    /// Already on the free list: a double free.
    AlreadyFree,
}

impl fmt::Display for FreeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            FreeError::NotManaged => f.write_str("frame is not managed by the allocator"),
            FreeError::NeverAllocated => f.write_str("frame was never allocated"),
            FreeError::AlreadyFree => f.write_str("frame is already free (double free)"),
        }
    }
}

/// A bump allocator over the usable regions, with an intrusive free list in
/// front of it.
///
/// The bump cursor only ever moves forward, so "has this frame ever been
/// handed out?" is a comparison against it, and a frame can only reach the
/// free list by having been allocated first.
pub struct FrameAllocator {
    regions: [FrameRange; MAX_REGIONS],
    region_count: usize,
    reservations: [Reservation; MAX_RESERVATIONS],
    reservation_count: usize,
    /// Region the bump cursor is working through.
    cursor_region: usize,
    /// Next frame index the bump cursor will consider. Frames below it in
    /// `cursor_region`, and every frame of every earlier region, have already
    /// been handed out (or skipped as reserved).
    cursor_frame: u32,
    /// Head of the intrusive free list, or [`NO_FRAME`].
    free_list: u32,
    /// Length of that list, which also bounds how far [`FrameAllocator::on_free_list`]
    /// will walk it.
    free_count: usize,
    /// Frames the pool holds in total, reservations already subtracted.
    total: usize,
    /// Frames currently handed out.
    in_use: usize,
}

impl FrameAllocator {
    const fn empty() -> FrameAllocator {
        FrameAllocator {
            regions: [FrameRange::EMPTY; MAX_REGIONS],
            region_count: 0,
            reservations: [Reservation {
                range: FrameRange::EMPTY,
                reason: "",
            }; MAX_RESERVATIONS],
            reservation_count: 0,
            cursor_region: 0,
            cursor_frame: 0,
            // Also the reason this static lands in `.data` rather than `.bss`:
            // the flat-binary loader in `boot.asm` writes no bytes for `.bss`,
            // so a static that starts out all-zero starts out as whatever the
            // machine left in that RAM. A non-zero field keeps the whole
            // static - the lock byte in front of it included - in the part of
            // the image that is actually loaded.
            free_list: NO_FRAME,
            free_count: 0,
            total: 0,
            in_use: 0,
        }
    }

    /// Builds the pool: every usable region of `map` that this CPU can
    /// address, minus everything already spoken for.
    fn new(map: &MemoryMap) -> FrameAllocator {
        let mut allocator = FrameAllocator::empty();

        allocator.reserve(
            0,
            BIOS_LOW_MEMORY_END,
            "IVT, BIOS data area, E820 handoff block",
        );
        // Nothing runs here once the CPU is in protected mode, but the boot
        // sector's own image is what a postmortem of a boot-time failure
        // would want to read, and one frame is a cheap price for keeping it.
        allocator.reserve(
            BOOT_SECTOR_PAGE,
            BOOT_SECTOR_PAGE + FRAME_SIZE,
            "boot sector image and real-mode stack",
        );
        let (kernel_start, kernel_end) = kernel_image();
        allocator.reserve(kernel_start, kernel_end, "kernel image, .bss and stack");
        allocator.reserve(
            LEGACY_ROM_START,
            LEGACY_ROM_END,
            "legacy video memory and BIOS ROM",
        );

        for region in map.regions().iter().filter(|region| region.is_usable()) {
            if allocator.region_count == MAX_REGIONS {
                break;
            }
            let range = FrameRange::inside(region.base, region.end());
            if range.is_empty() {
                continue;
            }
            allocator.regions[allocator.region_count] = range;
            allocator.region_count += 1;
        }

        if allocator.region_count > 0 {
            allocator.cursor_frame = allocator.regions[0].start;
        }
        allocator.total = allocator.count_allocatable();
        allocator
    }

    fn reserve(&mut self, start: u32, end: u32, reason: &'static str) {
        if self.reservation_count == MAX_RESERVATIONS {
            return;
        }
        self.reservations[self.reservation_count] = Reservation {
            range: FrameRange::covering(start, end),
            reason,
        };
        self.reservation_count += 1;
    }

    /// Whether any reservation covers this frame. Reservations are allowed to
    /// overlap each other (the kernel growing into one of the fixed ranges
    /// would be a bigger problem than double-counting), so this is a plain
    /// scan rather than anything that assumes they are disjoint.
    fn is_reserved(&self, index: u32) -> bool {
        self.reservations[..self.reservation_count]
            .iter()
            .any(|reservation| reservation.range.contains(index))
    }

    /// Which region holds this frame, if any.
    fn region_of(&self, index: u32) -> Option<usize> {
        self.regions[..self.region_count]
            .iter()
            .position(|region| region.contains(index))
    }

    /// Counted once, at init: the bump cursor cannot say how many frames it
    /// will end up skipping without walking them. Even 4 GiB of RAM is only a
    /// million iterations of a handful of comparisons, once, at boot.
    fn count_allocatable(&self) -> usize {
        let mut count = 0;
        for region in &self.regions[..self.region_count] {
            for index in region.start..region.end {
                if !self.is_reserved(index) {
                    count += 1;
                }
            }
        }
        count
    }

    /// Whether the bump cursor has already passed this frame, i.e. whether it
    /// could ever have been given to anyone.
    fn handed_out(&self, index: u32) -> bool {
        match self.region_of(index) {
            Some(region) => match region.cmp(&self.cursor_region) {
                Ordering::Less => true,
                Ordering::Equal => index < self.cursor_frame,
                Ordering::Greater => false,
            },
            None => false,
        }
    }

    fn allocate(&mut self) -> Option<Frame> {
        if let Some(frame) = self.pop_free() {
            self.in_use += 1;
            return Some(frame);
        }

        while self.cursor_region < self.region_count {
            let region = self.regions[self.cursor_region];
            if self.cursor_frame >= region.end {
                self.cursor_region += 1;
                if self.cursor_region < self.region_count {
                    self.cursor_frame = self.regions[self.cursor_region].start;
                }
                continue;
            }
            let index = self.cursor_frame;
            self.cursor_frame += 1;
            if self.is_reserved(index) {
                continue;
            }
            self.in_use += 1;
            return Some(Frame::from_index(index));
        }
        None
    }

    /// See [`allocate_contiguous`]: a contiguous run carved from the bump
    /// cursor, never from the free list.
    fn allocate_contiguous(&mut self, count: usize) -> Option<Frame> {
        if count == 0 {
            return None;
        }
        let count_u32 = count as u32;

        while self.cursor_region < self.region_count {
            let region = self.regions[self.cursor_region];
            if self.cursor_frame >= region.end {
                self.cursor_region += 1;
                if self.cursor_region < self.region_count {
                    self.cursor_frame = self.regions[self.cursor_region].start;
                }
                continue;
            }

            // Skip a reserved frame at the cursor so the run starts on real
            // allocatable memory.
            if self.is_reserved(self.cursor_frame) {
                self.cursor_frame += 1;
                continue;
            }

            let start = self.cursor_frame;
            // A contiguous run cannot cross a region boundary: the next
            // region's first frame may be megabytes away.
            if start.saturating_add(count_u32) > region.end {
                self.cursor_region += 1;
                if self.cursor_region < self.region_count {
                    self.cursor_frame = self.regions[self.cursor_region].start;
                }
                continue;
            }

            // Any reserved frame inside the candidate window breaks it; jump
            // past that frame and try again.
            if let Some(reserved) = (start..start + count_u32).find(|&i| self.is_reserved(i)) {
                self.cursor_frame = reserved + 1;
                continue;
            }

            self.cursor_frame = start + count_u32;
            self.in_use += count;
            return Some(Frame::from_index(start));
        }
        None
    }

    fn free(&mut self, frame: Frame) -> Result<(), FreeError> {
        let index = frame.index();
        if self.region_of(index).is_none() || self.is_reserved(index) {
            return Err(FreeError::NotManaged);
        }
        if !self.handed_out(index) {
            return Err(FreeError::NeverAllocated);
        }
        if self.on_free_list(index) {
            return Err(FreeError::AlreadyFree);
        }
        self.push_free(index);
        // Saturating because a wrong count is a reporting bug, while the
        // underflow panic it would otherwise cause here is a dead kernel.
        self.in_use = self.in_use.saturating_sub(1);
        Ok(())
    }

    fn push_free(&mut self, index: u32) {
        // SAFETY: `index` names a frame that came out of this allocator, so
        // it is usable RAM inside a region the BIOS reported, no reservation
        // covers it, and nothing else owns it while it is free. Frame 0 is
        // always reserved, so `link_of` is never null. Volatile because the
        // bytes are being used as allocator metadata rather than as anything
        // the compiler can reason about.
        unsafe { ptr::write_volatile(link_of(index), self.free_list) };
        self.free_list = index;
        self.free_count += 1;
    }

    fn pop_free(&mut self) -> Option<Frame> {
        if self.free_list == NO_FRAME {
            return None;
        }
        let index = self.free_list;
        // SAFETY: as for `push_free` - this frame is on the free list, so it
        // is one this allocator owns and last wrote a link into.
        self.free_list = unsafe { ptr::read_volatile(link_of(index)) };
        self.free_count = self.free_count.saturating_sub(1);
        Some(Frame::from_index(index))
    }

    /// Walks the free list looking for `index`, never taking more steps than
    /// the list is long: a corrupted link must come back as "not found"
    /// rather than as a kernel that never returns from a free.
    fn on_free_list(&self, index: u32) -> bool {
        let mut current = self.free_list;
        for _ in 0..self.free_count {
            if current == NO_FRAME {
                return false;
            }
            if current == index {
                return true;
            }
            // SAFETY: as for `push_free`.
            current = unsafe { ptr::read_volatile(link_of(current)) };
        }
        false
    }

    fn report(&self) -> Report {
        Report {
            total: self.total,
            in_use: self.in_use,
            regions: self.region_count,
            reservations: self.reservations,
            reservation_count: self.reservation_count,
        }
    }
}

/// Address of the first four bytes of a frame, where the free list keeps its
/// link.
fn link_of(index: u32) -> *mut u32 {
    (index << FRAME_SHIFT) as *mut u32
}

extern "C" {
    static __kernel_start: u8;
    static __kernel_end: u8;
}

/// Physical extent of the loaded kernel, from the linker script's own
/// symbols. `__kernel_end` is past `.bss`, which the flat-binary loader never
/// writes but the kernel very much occupies - the 64 KiB stack is in there.
///
/// Only the *addresses* of the two symbols are ever taken - they have no
/// storage of their own, so reading them would be meaningless (and unsafe).
fn kernel_image() -> (u32, u32) {
    (
        ptr::addr_of!(__kernel_start) as u32,
        ptr::addr_of!(__kernel_end) as u32,
    )
}

/// The pool, once [`init`] has built it. See [`FrameAllocator::empty`] for
/// why the initial value is not all-zero.
static ALLOCATOR: Mutex<FrameAllocator> = Mutex::new(FrameAllocator::empty());

/// What the allocator is made of and how much of it is spoken for.
#[derive(Clone, Copy)]
pub struct Report {
    /// Frames in the pool.
    pub total: usize,
    /// Frames currently handed out.
    pub in_use: usize,
    /// Usable regions the pool spans.
    pub regions: usize,
    reservations: [Reservation; MAX_RESERVATIONS],
    reservation_count: usize,
}

impl Report {
    /// Total size of the pool.
    pub fn total_bytes(&self) -> u64 {
        self.total as u64 * FRAME_SIZE as u64
    }

    /// The ranges excluded from the pool, in the order they were reserved.
    pub fn reservations(&self) -> &[Reservation] {
        &self.reservations[..self.reservation_count]
    }
}

impl fmt::Display for Report {
    /// The one-line summary for the boot banner, loud about the case where
    /// there is nothing to allocate: every later piece of memory management
    /// depends on this pool, so an empty one has to be reported here rather
    /// than discovered as an inexplicable failure in paging.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.total == 0 {
            return f.write_str(
                "Frames: NO ALLOCATABLE MEMORY (no usable region survived the reservations)",
            );
        }
        write!(
            f,
            "Frames: {} x 4 KiB allocatable ({} MiB) over {} regions, {} in use",
            self.total,
            self.total_bytes() / (1024 * 1024),
            self.regions,
            self.in_use
        )
    }
}

/// Builds the frame pool from the BIOS memory map.
///
/// Infallible, like the drivers: a machine whose map yields no usable frames
/// gets an empty pool that reports itself, which is a far better failure than
/// a kernel that will not boot at all.
pub fn init(map: &MemoryMap) -> Report {
    let mut allocator = ALLOCATOR.lock();
    // Assigning a whole freshly-built value, rather than mutating in place,
    // is deliberate: it leaves no field carrying over whatever was in memory
    // before (see `FrameAllocator::empty`).
    *allocator = FrameAllocator::new(map);
    allocator.report()
}

/// Takes one frame out of the pool. `None` means it is exhausted.
///
/// The frame's contents are whatever was in that RAM: the allocator does not
/// zero it, since the callers that need zeroed memory (page tables) know it
/// and the ones that don't (a heap) would pay for nothing.
pub fn allocate() -> Option<Frame> {
    ALLOCATOR.lock().allocate()
}

/// Takes `count` contiguous frames out of the pool. Returns the first frame
/// of the run, or `None` if no such run is left.
///
/// Unlike [`allocate`], this ignores the free list and only advances the bump
/// cursor: a heap (and anything else that needs one solid physical range)
/// cannot glue together the scattered frames a free list hands back. Frames
/// already on the free list stay there for single-frame callers.
pub fn allocate_contiguous(count: usize) -> Option<Frame> {
    ALLOCATOR.lock().allocate_contiguous(count)
}

/// Puts a frame back. Errors are the caller's bookkeeping mistakes, described
/// by [`FreeError`], and are reported rather than ignored.
pub fn free(frame: Frame) -> Result<(), FreeError> {
    ALLOCATOR.lock().free(frame)
}

/// What the pool currently looks like.
pub fn report() -> Report {
    ALLOCATOR.lock().report()
}

/// How many frames [`self_test`] takes out and puts back.
pub const SELF_TEST_FRAMES: usize = 8;

// The test frees two frames and re-allocates one of them, so it needs at
// least that many to work with.
const _: () = assert!(SELF_TEST_FRAMES >= 3);

/// The outcome of [`self_test`], including the addresses it was handed, so
/// the boot output can show the actual frames rather than just a verdict.
pub struct SelfTest {
    frames: [Frame; SELF_TEST_FRAMES],
    count: usize,
    freed: [Frame; 2],
    freed_count: usize,
    reused: Option<Frame>,
    failure: Option<&'static str>,
}

impl SelfTest {
    fn new() -> SelfTest {
        SelfTest {
            frames: [Frame::from_index(0); SELF_TEST_FRAMES],
            count: 0,
            freed: [Frame::from_index(0); 2],
            freed_count: 0,
            reused: None,
            failure: None,
        }
    }

    fn fail(&mut self, reason: &'static str) {
        if self.failure.is_none() {
            self.failure = Some(reason);
        }
    }

    /// The frames the test was handed, in the order it got them.
    pub fn frames(&self) -> &[Frame] {
        &self.frames[..self.count]
    }

    /// The frames it gave back before asking for one more.
    pub fn freed(&self) -> &[Frame] {
        &self.freed[..self.freed_count]
    }

    /// The frame the allocator handed out again after those frees.
    pub fn reused(&self) -> Option<Frame> {
        self.reused
    }
}

impl fmt::Display for SelfTest {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.failure {
            Some(reason) => write!(f, "Frames: SELF-TEST FAILED - {}", reason),
            None => write!(
                f,
                "Frames: self-test ok - {} distinct, LIFO reuse, double free refused",
                self.count
            ),
        }
    }
}

/// Exercises the allocator on every boot: takes [`SELF_TEST_FRAMES`] frames,
/// checks they are real and distinct, gives two back, checks the next
/// allocation reuses the most recently freed one, checks a double free is
/// refused, and returns everything.
///
/// Cheap enough to run unconditionally (a few dozen operations), and worth it:
/// a frame allocator that hands the same frame to two owners is exactly the
/// bug that would otherwise surface much later as inexplicable corruption
/// somewhere else entirely.
pub fn self_test() -> SelfTest {
    let mut test = SelfTest::new();
    let in_use_before = report().in_use;

    for _ in 0..SELF_TEST_FRAMES {
        match allocate() {
            Some(frame) => {
                test.frames[test.count] = frame;
                test.count += 1;
            }
            None => {
                test.fail("allocator ran out of frames");
                return test;
            }
        }
    }

    // A local copy, so the checks below can hold the allocator's lock without
    // also borrowing the report they are writing into.
    let frames = test.frames;
    {
        let allocator = ALLOCATOR.lock();
        for (position, frame) in frames[..test.count].iter().enumerate() {
            if allocator.region_of(frame.index()).is_none() {
                test.fail("handed out a frame outside usable RAM");
            }
            if allocator.is_reserved(frame.index()) {
                test.fail("handed out a reserved frame");
            }
            if frames[..position].contains(frame) {
                test.fail("handed out the same frame twice");
            }
        }
    }

    // Give the last two back, newest first, so the next allocation should
    // return the *second* of them: the free list is a stack, and an allocator
    // that answered with anything else would not be reading the links back at
    // all.
    for offset in 1..=2 {
        let frame = frames[test.count - offset];
        if free(frame).is_err() {
            test.fail("refused to take back a frame it had handed out");
            return test;
        }
        test.freed[test.freed_count] = frame;
        test.freed_count += 1;
    }
    let last_freed = test.freed[test.freed_count - 1];
    let still_free = test.freed[0];

    match allocate() {
        Some(frame) => {
            test.reused = Some(frame);
            if frame != last_freed {
                test.fail("did not reuse the most recently freed frame");
            }
        }
        None => test.fail("had nothing to allocate after two frames were freed"),
    }

    // The other one is still on the free list, so freeing it again has to be
    // refused rather than linking it in twice - which would hand the same
    // frame to two owners later on.
    if free(still_free) != Err(FreeError::AlreadyFree) {
        test.fail("accepted a double free");
    }

    for frame in &frames[..test.count] {
        if *frame == still_free {
            continue;
        }
        if free(*frame).is_err() {
            test.fail("refused to take back a frame it had handed out");
        }
    }

    if report().in_use != in_use_before {
        test.fail("frames went missing: the in-use count did not return to where it started");
    }

    test
}
