//! Physical memory: what exists, and (eventually) who owns it.
//!
//! Nothing here manages memory yet - this is the first piece, the map of what
//! the machine actually has. Everything above it (a frame allocator, paging, a
//! kernel heap) needs to know which physical addresses are real RAM and which
//! are firmware, memory-mapped devices, or simply not there, and the only
//! source for that is the BIOS - which the kernel cannot ask, because by the
//! time it runs the CPU is in protected mode and the BIOS is gone. See
//! [`map`] for how the bootloader asks on its behalf.

pub mod map;
