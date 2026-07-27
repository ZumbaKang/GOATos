//! Physical and virtual memory: what exists, who owns it, and how it is
//! addressed.
//!
//! [`map`] is what the machine has. Everything built on top of it needs to
//! know which physical addresses are real RAM and which are firmware,
//! memory-mapped devices, or simply not there, and the only source for that
//! is the BIOS - which the kernel cannot ask, because by the time it runs the
//! CPU is in protected mode and the BIOS is gone. See [`map`] for how the
//! bootloader asks on its behalf.
//!
//! [`frame`] is who owns it: the usable half of that map, minus what this
//! kernel has already put there, divided into 4 KiB frames that can be handed
//! out one at a time.
//!
//! [`paging`] is how it is addressed: a 32-bit non-PAE page directory and
//! page tables that identity-map low memory, with `CR0.PG` set so the CPU
//! actually uses them.
//!
//! [`heap`] is where smaller allocations come from: a contiguous run of
//! frames turned into a free-list heap and installed as the crate's
//! `#[global_allocator]`, so `alloc` (`Vec`, `Box`, `String`) works.
//!
//! [`layout`] is the written-down map of those pieces against the kernel
//! stacks and the unmapped guard page between them: ranges recorded at boot,
//! with a disjointness/adjacency check so a future change cannot silently put
//! the heap on top of a stack (or remap the guard).

pub mod frame;
pub mod heap;
pub mod layout;
pub mod map;
pub mod paging;
