---
name: memory-management
description: Guidance for implementing physical/virtual memory management in GOATos (paging, a physical frame allocator, a kernel heap) - not yet built. Use this when starting work on memory management, paging, or adding alloc/Vec/Box support to the kernel.
---

# Memory management (barely started)

GOATos currently runs entirely without paging: the bootloader (see
`bootloader-and-linking`) leaves the CPU with a flat, unpaged 32-bit
protected-mode address space, and the kernel only ever touches a handful of
fixed, hand-picked addresses (the load address `0x10000`, the VGA buffer
`0xb8000`, the 64KiB stack in `.bss`). There is no heap, no `alloc` crate,
no `Vec`/`Box`/`String` - only `core`.

The one piece that does exist is the **memory map** (step 1 below).

## Suggested order of implementation

1. **Get a memory map.** *Done* - `detect_memory` in `boot/boot.asm` walks
   `INT 15h, EAX=0xE820` in real mode and writes the raw entries to a fixed
   low-memory address; `kernel/src/memory/map.rs` reads them back. Things
   worth knowing before building on it:
   - The handoff block lives at **0x500** and is `u32` signature (`"GOAT"`),
     `u32` entry count, then up to 32 packed 24-byte entries (`u64` base,
     `u64` length, `u32` type, `u32` ACPI 3.0 attributes). Both ends of that
     layout are constants - keep `MEMMAP_*` in `boot.asm` and the consts in
     `map.rs` in step. It ends at 0x808, clear of the real-mode stack that
     grows down from 0x7c00.
   - The signature matters: low memory is *not* guaranteed to be zero on real
     hardware, so "the count looks plausible" is not evidence a map was
     handed over. A BIOS with no E820 support has to be reported, not guessed
     around.
   - E820 addresses are **64-bit** even on this 32-bit kernel (a PC's PCI
     hole is remapped above 4GiB), and `map.rs` keeps them as reported. A
     frame allocator is where clamping to what this CPU can address belongs.
   - `map.rs` drops zero-length entries and ones whose ACPI attribute bit 0
     is clear; entry *types* other than 1 are all treated as unusable,
     including unknown ones.
   - The usable total is a cheap sanity check on the whole chain, because it
     tracks the emulator's configured RAM: 127 MiB under QEMU's 128MiB
     default, 31 MiB under the web demo's 32MiB v86. `scripts/ci-test.sh`
     asserts the QEMU figure.
2. **A physical frame allocator.** Simplest reasonable starting point: a
   bump/free-list allocator over the usable regions from the memory map,
   operating in 4KiB frames.
3. **Paging.** Set up a page directory + page tables (32-bit, non-PAE, so
   two-level paging: page directory -> page table -> 4KiB page) and enable
   it via `CR3`/`CR0.PG`. Consider identity-mapping low memory first (the
   simplest correct thing) before doing anything fancier like a higher-half
   kernel.
4. **A kernel heap + global allocator.** Once paging exists, add a
   `#[global_allocator]` (a simple bump or free-list heap allocator is
   fine to start) so `alloc` (and thus `Vec`, `Box`, `String`, etc.)
   becomes available. This unblocks almost everything after it (a real
   scheduler, filesystem, etc. all want dynamic allocation).

## Conventions to follow

- Keep new code in its own module (`kernel/src/memory/` or similar), not
  bolted onto `main.rs`.
- Anything that touches raw addresses/`unsafe` needs a `# Safety` doc
  comment explaining the invariant being relied on - see `vga.rs` and
  `serial.rs` for the existing style.
- Follow the defensive-driver pattern from `qemu-testing-and-verification`:
  a failure here shouldn't be able to silently hang the kernel with no
  output. A `panic!` with a clear message beats a silent freeze.
- The [OSDev wiki](https://osdev.wiki/) pages on "Memory Management",
  "Page Frame Allocation", and "Paging" are the standard references for
  this stage of a hobby OS.
