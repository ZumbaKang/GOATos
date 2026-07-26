---
name: memory-management
description: Guidance for implementing physical/virtual memory management in GOATos (paging, a physical frame allocator, a kernel heap) - not yet built. Use this when starting work on memory management, paging, or adding alloc/Vec/Box support to the kernel.
---

# Memory management (not yet implemented)

GOATos currently runs entirely without paging: the bootloader (see
`bootloader-and-linking`) leaves the CPU with a flat, unpaged 32-bit
protected-mode address space, and the kernel only ever touches a handful of
fixed, hand-picked addresses (the load address `0x10000`, the VGA buffer
`0xb8000`, the 64KiB stack in `.bss`). There is no heap, no `alloc` crate,
no `Vec`/`Box`/`String` - only `core`.

## Suggested order of implementation

1. **Get a memory map.** Right now the bootloader doesn't hand the kernel
   any information about available RAM. Before building a real allocator,
   `boot/boot.asm` needs to query the BIOS memory map (`INT 15h, EAX=0xE820`
   is the standard, well-supported way) and pass the resulting list to the
   kernel (e.g. at a fixed, pre-agreed memory address, with a fixed-size
   struct/array the kernel reads on startup) - similar in spirit to how it
   already passes control to `kernel_main`, but for memory regions instead
   of code.
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
