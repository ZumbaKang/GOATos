//! Where the kernel's stacks, stack guard page, and heap live, written down
//! so they cannot silently collide as either side grows.
//!
//! ## Address map (identity-mapped: virtual == physical)
//!
//! ```text
//! [__kernel_start, __kernel_end)   whole loaded image, reserved from the
//!                                  frame allocator. Contains .text /
//!                                  .rodata / .data / .bss. The stacks and
//!                                  guard live in .bss, laid out by entry.s:
//!
//!   [df_stack_bottom, df_stack_top)  4 KiB  double-fault handler stack
//!   [stack_guard,     stack_bottom)  4 KiB  unmapped (paging leaves PTE
//!                                           not-present)
//!   [stack_bottom,    stack_top)    64 KiB  ordinary kernel stack; grows
//!                                           down from stack_top
//!
//! [heap_start, heap_end)           1 MiB contiguous frames from
//!                                  frame::allocate_contiguous, always
//!                                  outside the reserved kernel image.
//! ```
//!
//! The stacks cannot grow into the heap by construction: the frame allocator
//! never hands out a frame inside `__kernel_start..__kernel_end`, and the heap
//! only comes from those handed-out frames. A kernel-stack overflow cannot
//! reach the double-fault stack either: the unmapped guard page between them
//! makes the access fault (and escalate to a double fault on the private
//! stack) instead of scribbling through.
//!
//! This module records the ranges at boot and refuses to continue quietly if
//! that invariant is broken - a future change that moves either side, drops
//! the guard, or remaps the guard page gets a loud failure instead of silent
//! corruption.

use core::fmt;

use crate::tss;

use super::heap;
use super::paging;

/// Bytes reserved for the ordinary kernel stack in `entry.s`. Must match the
/// `.skip` there; [`check`] verifies the linker symbols agree at boot.
pub const KERNEL_STACK_SIZE: usize = 64 * 1024;

/// Bytes reserved for the double-fault handler's private stack in `entry.s`.
/// Named here so the layout report can print it without opening that module's
/// internals; [`check`] verifies the live range matches.
pub const DOUBLE_FAULT_STACK_SIZE: usize = 4096;

/// Bytes left unmapped immediately below the kernel stack. One page - enough
/// that any access from an overflowing `esp` hits a not-present PTE.
pub const GUARD_PAGE_SIZE: usize = 4096;

extern "C" {
    static stack_bottom: u8;
    static stack_top: u8;
    static stack_guard_page: u8;
    static __kernel_start: u8;
    static __kernel_end: u8;
}

/// A half-open byte range `[start, end)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub start: usize,
    pub end: usize,
}

impl Range {
    pub const fn new(start: usize, end: usize) -> Range {
        Range { start, end }
    }

    pub const fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub const fn is_empty(self) -> bool {
        self.end <= self.start
    }

    /// Whether `self` and `other` share any byte. Empty ranges never overlap.
    pub const fn overlaps(self, other: Range) -> bool {
        if self.is_empty() || other.is_empty() {
            return false;
        }
        self.start < other.end && other.start < self.end
    }

    /// Whether every byte of `other` lies inside `self`.
    pub const fn contains_range(self, other: Range) -> bool {
        !other.is_empty() && other.start >= self.start && other.end <= self.end
    }
}

impl fmt::Display for Range {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:#010x}-{:#010x}", self.start, self.end)
    }
}

/// The live ranges [`check`] measured, plus whether they are still disjoint.
pub struct Report {
    pub kernel_image: Range,
    pub kernel_stack: Range,
    pub double_fault_stack: Range,
    pub guard_page: Range,
    pub heap: Range,
    failure: Option<&'static str>,
}

impl Report {
    /// `None` when every named range is where it should be.
    pub fn failure(&self) -> Option<&'static str> {
        self.failure
    }

    pub fn ok(&self) -> bool {
        self.failure.is_none()
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if let Some(reason) = self.failure {
            return write!(f, "Layout: OVERLAP/MISMATCH - {}", reason);
        }
        write!(
            f,
            "Layout: DF stack {} ({} KiB), guard {} ({} KiB, unmapped), kernel stack {} ({} KiB), heap {} ({} KiB) - ok",
            self.double_fault_stack,
            self.double_fault_stack.len() / 1024,
            self.guard_page,
            self.guard_page.len() / 1024,
            self.kernel_stack,
            self.kernel_stack.len() / 1024,
            self.heap,
            self.heap.len() / 1024
        )
    }
}

/// Reads the live stack/guard/heap/image ranges and checks the invariants this
/// module documents. Cheap enough to run on every boot; the printed report is
/// what keeps the layout from living only in someone's head.
pub fn check(heap: heap::Report) -> Report {
    // Only the *addresses* of these linker/assembly symbols are taken - they
    // name objects that exist for the life of the kernel, and `addr_of!` does
    // not read through them (same pattern as `frame::kernel_image`).
    let kernel_image = Range::new(
        core::ptr::addr_of!(__kernel_start) as usize,
        core::ptr::addr_of!(__kernel_end) as usize,
    );
    let kernel_stack = Range::new(
        core::ptr::addr_of!(stack_bottom) as usize,
        core::ptr::addr_of!(stack_top) as usize,
    );
    let guard_start = core::ptr::addr_of!(stack_guard_page) as usize;
    let guard_page = Range::new(guard_start, guard_start + GUARD_PAGE_SIZE);
    let (df_bottom, df_top) = tss::double_fault_stack_range();
    let double_fault_stack = Range::new(df_bottom as usize, df_top as usize);
    let heap_range = if heap.ready() {
        Range::new(heap.start(), heap.end())
    } else {
        Range::new(0, 0)
    };

    let mut report = Report {
        kernel_image,
        kernel_stack,
        double_fault_stack,
        guard_page,
        heap: heap_range,
        failure: None,
    };

    if kernel_stack.len() != KERNEL_STACK_SIZE {
        report.failure = Some("kernel stack size does not match KERNEL_STACK_SIZE");
        return report;
    }
    if double_fault_stack.len() != DOUBLE_FAULT_STACK_SIZE {
        report.failure = Some("double-fault stack size does not match DOUBLE_FAULT_STACK_SIZE");
        return report;
    }
    if guard_page.len() != GUARD_PAGE_SIZE {
        report.failure = Some("guard page size does not match GUARD_PAGE_SIZE");
        return report;
    }
    if !kernel_image.contains_range(kernel_stack) {
        report.failure = Some("kernel stack is outside the kernel image");
        return report;
    }
    if !kernel_image.contains_range(double_fault_stack) {
        report.failure = Some("double-fault stack is outside the kernel image");
        return report;
    }
    if !kernel_image.contains_range(guard_page) {
        report.failure = Some("guard page is outside the kernel image");
        return report;
    }
    // entry.s lays them out as DF stack | guard | kernel stack. Anything else
    // means the assembly layout drifted from what paging leaves unmapped.
    if double_fault_stack.end != guard_page.start {
        report.failure = Some("double-fault stack is not immediately below the guard page");
        return report;
    }
    if guard_page.end != kernel_stack.start {
        report.failure = Some("guard page is not immediately below the kernel stack");
        return report;
    }
    if kernel_stack.overlaps(double_fault_stack) {
        report.failure = Some("kernel stack and double-fault stack overlap");
        return report;
    }
    if kernel_stack.overlaps(guard_page) || double_fault_stack.overlaps(guard_page) {
        report.failure = Some("guard page overlaps a stack");
        return report;
    }
    // The whole point of the guard: the PTE must stay not-present.
    if paging::is_present(guard_page.start as u32) {
        report.failure = Some("stack guard page is still mapped");
        return report;
    }
    if !heap.ready() {
        // Heap init already reported FAILURE; don't also claim an overlap
        // against an empty range. Layout of the stacks is still checked above.
        return report;
    }
    if heap_range.len() != heap::HEAP_SIZE {
        report.failure = Some("heap size does not match HEAP_SIZE");
        return report;
    }
    if kernel_image.overlaps(heap_range) {
        report.failure = Some("heap overlaps the kernel image (and therefore a stack)");
        return report;
    }
    if kernel_stack.overlaps(heap_range) {
        report.failure = Some("kernel stack and heap overlap");
        return report;
    }
    if double_fault_stack.overlaps(heap_range) {
        report.failure = Some("double-fault stack and heap overlap");
        return report;
    }
    if guard_page.overlaps(heap_range) {
        report.failure = Some("guard page and heap overlap");
        return report;
    }

    report
}
