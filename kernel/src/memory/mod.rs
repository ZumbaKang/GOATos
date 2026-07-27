//! Physical memory: what exists, and who owns it.
//!
//! [`map`] is what the machine has. Everything built on top of it (paging, a
//! kernel heap) needs to know which physical addresses are real RAM and which
//! are firmware, memory-mapped devices, or simply not there, and the only
//! source for that is the BIOS - which the kernel cannot ask, because by the
//! time it runs the CPU is in protected mode and the BIOS is gone. See
//! [`map`] for how the bootloader asks on its behalf.
//!
//! [`frame`] is who owns it: the usable half of that map, minus what this
//! kernel has already put there, divided into 4 KiB frames that can be handed
//! out one at a time.
//!
//! Still missing above these two: paging, and a heap to make `alloc`
//! (`Vec`, `Box`, `String`) available.

pub mod frame;
pub mod map;
