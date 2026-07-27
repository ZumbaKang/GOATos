//! Kernel heap and global allocator: where `alloc` (`Vec`, `Box`, `String`)
//! gets its memory.
//!
//! [`super::frame`] hands out whole 4 KiB frames; most kernel data structures
//! want something smaller. This module carves a contiguous run of those
//! frames into a heap and exposes it as the crate's [`GlobalAlloc`], so the
//! rest of the kernel can use the collections in `alloc` without knowing
//! where the bytes came from.
//!
//! The allocator itself is a first-fit free list with coalescing. A freed
//! block stores the next-free pointer in its own header - the same intrusive
//! trick the frame allocator uses, just at byte granularity instead of 4 KiB.
//! Adjacent free blocks are merged on free so a churn of short-lived
//! allocations does not permanently fragment the heap into pieces too small
//! to satisfy the next request.
//!
//! Identity mapping means the heap's physical frames are already reachable at
//! the same addresses, so there is nothing virtual to set up beyond what
//! [`super::paging`] already did. Task 2.5 will pin the heap/stack layout in
//! writing; for now the boot banner prints the range this module claimed.

use core::alloc::{GlobalAlloc, Layout};
use core::fmt;
use core::ptr;

use spin::Mutex;

use super::frame::{self, FRAME_SIZE};

/// Bytes the heap claims from the frame allocator. One mebibyte is plenty for
/// early `Vec`/`Box` use and small enough that even the web demo's 32 MiB of
/// RAM still has most of its pool left for page tables and everything else.
pub const HEAP_SIZE: usize = 1024 * 1024;

const _: () = assert!(HEAP_SIZE % FRAME_SIZE as usize == 0);
const HEAP_FRAMES: usize = HEAP_SIZE / FRAME_SIZE as usize;

/// Every heap block - free or live - is aligned to this. Large enough for any
/// ordinary Rust type on this target (pointers, `u64`, most structs); a
/// request that needs stricter alignment is refused rather than silently
/// misaligned.
const ALIGN: usize = 8;

/// Bytes occupied by a free-block header (`size` + `next`). Also the smallest
/// block the free list will keep: anything smaller gets absorbed into its
/// neighbour on split/coalesce.
const HEADER_SIZE: usize = core::mem::size_of::<FreeBlock>();
const _: () = assert!(HEADER_SIZE == 8);
const _: () = assert!(HEADER_SIZE % ALIGN == 0);

/// Intrusive free-list node, stored at the start of every free block.
///
/// `size` is the whole block, header included. Live blocks keep the same
/// `size` field in the same place and leave the rest of the block to the
/// caller, so allocation returns `block + HEADER_SIZE`.
#[repr(C)]
struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

/// The free list plus the heap's fixed bounds. Empty until [`init`] fills it
/// in; allocations against an uninitialised heap return null.
struct Heap {
    heap_start: usize,
    heap_end: usize,
    free_list: *mut FreeBlock,
    /// Bytes currently handed out (payload + their headers).
    used: usize,
    /// Successful allocations since init - a cheap self-test counter.
    allocations: usize,
}

// SAFETY: every method that touches `free_list` / the blocks it names holds
// the `ALLOCATOR` mutex, so the raw pointers are never raced.
unsafe impl Send for Heap {}

impl Heap {
    const fn empty() -> Heap {
        Heap {
            // Non-zero so the surrounding `Mutex` static lands in `.data` and
            // is actually loaded - same reason as `FrameAllocator::empty`.
            heap_start: 1,
            heap_end: 1,
            free_list: ptr::null_mut(),
            used: 0,
            allocations: 0,
        }
    }

    fn is_ready(&self) -> bool {
        self.heap_end > self.heap_start
    }

    /// # Safety
    ///
    /// `start..start+size` must be a contiguous region of usable, identity-
    /// mapped RAM that nothing else will touch for the life of the heap.
    unsafe fn init(&mut self, start: usize, size: usize) {
        debug_assert!(size >= HEADER_SIZE);
        debug_assert!(start % ALIGN == 0);
        debug_assert!(size % ALIGN == 0);

        let block = start as *mut FreeBlock;
        // SAFETY: caller guarantees the region is ours and large enough for
        // one free-block header. The whole heap starts as a single free hole.
        unsafe {
            (*block).size = size;
            (*block).next = ptr::null_mut();
        }
        self.heap_start = start;
        self.heap_end = start + size;
        self.free_list = block;
        self.used = 0;
        self.allocations = 0;
    }

    fn allocate(&mut self, layout: Layout) -> *mut u8 {
        if !self.is_ready() || layout.size() == 0 {
            return ptr::null_mut();
        }
        if layout.align() > ALIGN {
            return ptr::null_mut();
        }

        let needed = match aligned_block_size(layout.size()) {
            Some(n) => n,
            None => return ptr::null_mut(),
        };

        // First-fit: take the first free block that can hold `needed`, and
        // split the tail back onto the free list when it is large enough to
        // be a free block of its own.
        let mut prev: *mut FreeBlock = ptr::null_mut();
        let mut current = self.free_list;
        while !current.is_null() {
            // SAFETY: every pointer on the free list was either placed by
            // `init` or by a prior free/split of a block inside this heap.
            let block = unsafe { &mut *current };
            let next = block.next;
            if block.size >= needed {
                let remaining = block.size - needed;
                if remaining >= HEADER_SIZE {
                    let split = (current as usize + needed) as *mut FreeBlock;
                    // SAFETY: `remaining` is at least a header and the split
                    // address lies inside the block we are carving up.
                    unsafe {
                        (*split).size = remaining;
                        (*split).next = next;
                    }
                    self.unlink(prev, split);
                } else {
                    // Tail too small to free on its own - keep it with the
                    // allocation so it is not lost.
                    self.unlink(prev, next);
                }
                // SAFETY: we are about to hand this block to the caller; size
                // stays, next is no longer meaningful.
                unsafe {
                    (*current).size = if remaining >= HEADER_SIZE {
                        needed
                    } else {
                        needed + remaining
                    };
                    (*current).next = ptr::null_mut();
                }
                let total = unsafe { (*current).size };
                self.used += total;
                self.allocations += 1;
                return (current as usize + HEADER_SIZE) as *mut u8;
            }
            prev = current;
            current = next;
        }
        ptr::null_mut()
    }

    fn deallocate(&mut self, ptr: *mut u8, _layout: Layout) {
        if ptr.is_null() || !self.is_ready() {
            return;
        }
        let block_addr = (ptr as usize) - HEADER_SIZE;
        if block_addr < self.heap_start || block_addr >= self.heap_end {
            return;
        }
        let block = block_addr as *mut FreeBlock;
        // SAFETY: `ptr` was returned by `allocate`, so the bytes immediately
        // before it are a header we wrote, and `size` still describes the
        // whole block.
        let size = unsafe { (*block).size };
        if size < HEADER_SIZE || block_addr + size > self.heap_end {
            return;
        }
        self.used = self.used.saturating_sub(size);
        self.insert_free(block, size);
    }

    fn unlink(&mut self, prev: *mut FreeBlock, new_next: *mut FreeBlock) {
        if prev.is_null() {
            self.free_list = new_next;
        } else {
            // SAFETY: `prev` is a free-list node we walked to reach the block
            // being unlinked.
            unsafe {
                (*prev).next = new_next;
            }
        }
    }

    /// Inserts `block` into the free list sorted by address, then merges it
    /// with any immediate neighbours so adjacent frees become one hole.
    fn insert_free(&mut self, block: *mut FreeBlock, size: usize) {
        // SAFETY: caller verified `block` lies inside the heap and `size` is
        // its whole extent.
        unsafe {
            (*block).size = size;
            (*block).next = ptr::null_mut();
        }

        let mut prev: *mut FreeBlock = ptr::null_mut();
        let mut current = self.free_list;
        while !current.is_null() && (current as usize) < (block as usize) {
            prev = current;
            // SAFETY: walking the free list as in `allocate`.
            current = unsafe { (*current).next };
        }

        // SAFETY: splicing into the sorted list; coalesce checks below keep
        // the size/next fields consistent.
        unsafe {
            (*block).next = current;
            if prev.is_null() {
                self.free_list = block;
            } else {
                (*prev).next = block;
            }

            // Merge with the following block when they touch.
            if !current.is_null() && (block as usize) + (*block).size == current as usize {
                (*block).size += (*current).size;
                (*block).next = (*current).next;
            }

            // Merge with the previous block when they touch.
            if !prev.is_null() && (prev as usize) + (*prev).size == block as usize {
                (*prev).size += (*block).size;
                (*prev).next = (*block).next;
            }
        }
    }
}

/// Bytes a live block needs for `payload` usable bytes: header, then the
/// payload rounded up so the next block stays [`ALIGN`]-aligned.
fn aligned_block_size(payload: usize) -> Option<usize> {
    let without_header = align_up(payload, ALIGN)?;
    without_header.checked_add(HEADER_SIZE).and_then(|n| align_up(n, ALIGN))
}

fn align_up(value: usize, align: usize) -> Option<usize> {
    let mask = align - 1;
    Some(value.checked_add(mask)? & !mask)
}

/// Spin-locked heap that implements [`GlobalAlloc`].
struct LockedHeap {
    inner: Mutex<Heap>,
}

impl LockedHeap {
    const fn empty() -> LockedHeap {
        LockedHeap {
            inner: Mutex::new(Heap::empty()),
        }
    }
}

// SAFETY: the mutex serialises every alloc/dealloc, and `Heap` only ever
// hands out pointers inside the region `init` was given.
unsafe impl GlobalAlloc for LockedHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.inner.lock().allocate(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.inner.lock().deallocate(ptr, layout);
    }
}

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// What [`init`] claimed and whether the boot self-test exercised it.
#[derive(Clone, Copy)]
pub struct Report {
    start: usize,
    size: usize,
    ready: bool,
}

impl Report {
    /// First byte of the heap, or 0 if init failed.
    pub fn start(self) -> usize {
        self.start
    }

    /// Bytes reserved for the heap.
    pub fn size(self) -> usize {
        self.size
    }

    /// Exclusive end address.
    pub fn end(self) -> usize {
        self.start.saturating_add(self.size)
    }

    /// Whether the global allocator has a backing region.
    pub fn ready(self) -> bool {
        self.ready
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if !self.ready {
            return f.write_str("Heap: FAILED (could not reserve a contiguous region)");
        }
        write!(
            f,
            "Heap: {:#010x}-{:#010x} ({} KiB), free-list allocator ready",
            self.start,
            self.end(),
            self.size / 1024
        )
    }
}

/// Outcome of [`self_test`]: a `Vec` push/read/drop cycle through the global
/// allocator.
pub struct SelfTest {
    pushed: usize,
    failure: Option<&'static str>,
}

impl SelfTest {
    /// How many bytes the test successfully pushed into its `Vec`.
    pub fn pushed(&self) -> usize {
        self.pushed
    }

    /// `None` on success.
    pub fn failure(&self) -> Option<&'static str> {
        self.failure
    }
}

impl fmt::Display for SelfTest {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.failure {
            Some(reason) => write!(f, "Heap: SELF-TEST FAILED - {}", reason),
            None => write!(
                f,
                "Heap: self-test ok - Vec pushed {} bytes and read them back",
                self.pushed
            ),
        }
    }
}

/// Reserves [`HEAP_SIZE`] bytes of contiguous frames and installs them as the
/// global allocator's backing store.
///
/// Infallible in the driver sense: a machine that cannot spare the frames
/// gets a loud `FAILED` report and a heap that still returns null on every
/// alloc, rather than a kernel that will not boot.
pub fn init() -> Report {
    let Some(start_frame) = frame::allocate_contiguous(HEAP_FRAMES) else {
        return Report {
            start: 0,
            size: 0,
            ready: false,
        };
    };
    let start = start_frame.start_address() as usize;
    // SAFETY: `allocate_contiguous` just handed us `HEAP_FRAMES` unused,
    // identity-mapped frames starting at `start`. Nothing else holds them.
    unsafe {
        ALLOCATOR.inner.lock().init(start, HEAP_SIZE);
    }
    Report {
        start,
        size: HEAP_SIZE,
        ready: true,
    }
}

/// How many bytes [`self_test`] pushes into its `Vec`.
pub const SELF_TEST_BYTES: usize = 64;

/// Exercises the global allocator on every boot: builds a `Vec<u8>`, pushes
/// a known pattern, reads it back, and drops it (which must free without
/// faulting). Running unconditionally is cheap and is what proves `alloc`
/// actually works - a banner that only says the heap was *reserved* would
/// not catch a broken free list.
pub fn self_test() -> SelfTest {
    use alloc::vec::Vec;

    let mut test = SelfTest {
        pushed: 0,
        failure: None,
    };

    if !ALLOCATOR.inner.lock().is_ready() {
        test.failure = Some("heap was not initialised");
        return test;
    }

    let used_before = ALLOCATOR.inner.lock().used;
    let mut vec: Vec<u8> = Vec::new();
    for i in 0..SELF_TEST_BYTES {
        vec.push(i as u8);
        test.pushed += 1;
    }

    if vec.len() != SELF_TEST_BYTES {
        test.failure = Some("Vec length mismatch after pushes");
        return test;
    }
    for (i, byte) in vec.iter().enumerate() {
        if *byte != i as u8 {
            test.failure = Some("Vec contents did not match what was pushed");
            return test;
        }
    }

    // Grow past the inline/small capacity so the allocator has to hand out a
    // second block and then free the first - exercises both paths.
    vec.resize(SELF_TEST_BYTES * 4, 0xA5);
    if vec.len() != SELF_TEST_BYTES * 4 {
        test.failure = Some("Vec resize failed");
        return test;
    }
    for (i, byte) in vec.iter().enumerate().take(SELF_TEST_BYTES) {
        if *byte != i as u8 {
            test.failure = Some("Vec lost data across resize");
            return test;
        }
    }

    drop(vec);

    let heap = ALLOCATOR.inner.lock();
    if heap.used != used_before {
        test.failure = Some("bytes went missing: used count did not return after drop");
    }
    test
}
