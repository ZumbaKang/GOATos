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
//! Still missing above these three: a heap to make `alloc` (`Vec`, `Box`,
//! `String`) available.

pub mod frame;
pub mod map;
pub mod paging;
