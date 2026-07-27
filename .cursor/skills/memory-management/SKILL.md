---
name: memory-management
description: Guidance for implementing physical/virtual memory management in GOATos (a BIOS memory map, physical frame allocator, and identity-mapped paging exist; a kernel heap is not built yet). Use this when starting work on memory management, paging, or adding alloc/Vec/Box support to the kernel.
---

# Memory management (paging on, no heap yet)

GOATos boots with paging off (the bootloader - see `bootloader-and-linking` -
leaves a flat 32-bit protected-mode address space), then
`kernel/src/memory/paging.rs` identity-maps low memory and sets `CR0.PG`.
After that every address is still numerically equal to its physical frame,
but only because the page tables say so. There is no heap, no `alloc` crate,
no `Vec`/`Box`/`String` - only `core`.

What does exist is the **memory map**, a **physical frame allocator**, and
**identity-mapped paging** (steps 1-3 below). A kernel heap is the next step.

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
2. **A physical frame allocator.** *Done* - `kernel/src/memory/frame.rs` is a
   bump cursor over the usable regions with an intrusive free list in front of
   it, in 4KiB frames. What to know before building on it:
   - `frame::allocate()` hands out a `Frame`, `frame::free()` takes one back
     and *reports* a double free rather than accepting it, and
     `frame::report()` is the summary the boot banner prints. Frames come back
     un-zeroed; page tables zero their own.
   - The free list lives **inside the free frames** (first four bytes = index
     of the next free frame). So a frame handed out has whatever the last
     owner (or the allocator) left in it, and nothing may write to a frame it
     has freed.
   - Usable-per-E820 is not the same as free. The allocator subtracts a fixed
     list of reservations: the IVT/BDA plus the E820 handoff block
     (0x0-0x1000), the boot sector's page (0x7000), the kernel image
     (`__kernel_start`/`__kernel_end`, two symbols `linker.ld` exports - they
     span `.bss`, so the 64KiB kernel stack and the double-fault stack are
     both inside), and 0xa0000-0x100000. **Anything new that claims a fixed
     physical address must be added there**, or the allocator will hand it to
     someone else.
   - Addresses above the 4GiB line are dropped, since this CPU cannot reach
     them. Frames are named by index, not address, so the top frame is still
     representable.
   - The pool size tracks the emulator, like the map it comes from: 32576
     frames (127 MiB) under QEMU's 128MiB default, 8032 (31 MiB) under the web
     demo's 32MiB v86. `scripts/ci-test.sh` asserts the QEMU figure, and
     independently re-checks the frame addresses the boot self-test prints
     against the reserved ranges it prints.
3. **Paging.** *Done* - `kernel/src/memory/paging.rs` builds a 32-bit non-PAE
   page directory + page tables (directory -> table -> 4KiB page),
   identity-maps `0..` the top of usable RAM rounded up to 4 MiB (holes
   included, so VGA at 0xb8000 stays reachable), writes that directory into
   both TSSes via `tss::set_page_directory`, then loads `CR3` and sets
   `CR0.PG`. Things worth knowing before building on it:
   - Page directory and each page table cost one frame from `frame::allocate`,
     zeroed first. Under QEMU's 128 MiB that is 1 + 32 frames; under v86's
     32 MiB, 1 + 8.
   - **Both TSSes carry the real `cr3`.** A double-fault task switch loads
     `CR3` from the incoming TSS; leaving it at 0 would switch onto a
     directory of zeroes and triple-fault.
   - Identity mapping is deliberate and temporary: virtual == physical for
     everything the kernel touches, which is why existing code keeps running
     after `CR0.PG` flips. A higher-half kernel would remake this.
   - 2.6 (guard page below the kernel stack) is the next paging-shaped task;
     it needs an unmapped page, so resist the urge to keep "map everything
     forever" as an invariant.
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
