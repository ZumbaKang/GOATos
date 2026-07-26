---
name: filesystem
description: Guidance for adding disk-backed storage/filesystem support to GOATos - not yet built, and blocked on a real disk driver and (for anything beyond trivial use) memory management. Use this when starting work on a filesystem, VFS, or persistent storage.
---

# Filesystem (not yet implemented)

GOATos currently has no concept of files: the disk is only ever used
implicitly, to boot (see `bootloader-and-linking`). There is no filesystem
driver, no VFS layer, nothing.

## Suggested order of implementation

1. **A real kernel-side disk driver.** `boot/boot.asm` has a working
   CHS-based BIOS disk reader, but it's assembly, boot-time-only, and BIOS
   calls aren't available once the kernel is running in pure protected
   mode with no callback path set up. A real driver needs to talk to the
   ATA/PIO (or eventually AHCI) controller directly via I/O ports - see the
   `drivers` skill for where this fits.
2. **Pick (or design) a simple on-disk format.** For a hobby OS, a minimal
   custom format or a well-documented one like FAT16/FAT32 are both
   reasonable starting points; FAT has the advantage that images are easy
   to inspect/populate from a normal desktop OS while developing.
3. **A minimal VFS abstraction** (even something as simple as an `open`/
   `read`/`write`/`close`-style trait) before wiring in a second
   filesystem type, so the rest of the kernel doesn't end up hard-coded to
   one on-disk format.
4. Consider whether GOATos needs a **ramdisk/initrd** at all before a real
   disk driver exists - it can be a useful intermediate step (a filesystem
   image embedded directly in the kernel binary or loaded by the
   bootloader) to unblock VFS/API design work without needing the ATA
   driver finished first.

## Conventions to follow

- This will very likely be the first thing in GOATos that needs the heap
  (`alloc`/`Vec`/`Box`) - make sure `memory-management` has a working
  global allocator before going far here.
- Disk I/O errors must be handled, not `unwrap()`'d - see the
  defensive-driver pattern in `qemu-testing-and-verification` and
  `drivers`.
- If/when this needs testing against real files, prefer building small,
  purpose-made disk images (e.g. via a `scripts/` helper, following the
  spirit of `scripts/build-web-demo.sh`) over hand-crafting bytes in Rust
  test code.
