---
name: filesystem
description: Guidance for adding disk-backed storage/filesystem support to GOATos. A minimal ATA PIO driver and custom GOATFS (enough for shell `cat`) exist; use this when extending storage, adding write support, or replacing GOATFS with something richer.
---

# Filesystem

GOATos can read files from its disk image. Roadmap 4.5 landed the first cut:

- **`kernel/src/ata.rs`** — polling PIO driver for the primary ATA master
  (28-bit LBA reads only). Soft-fails on a missing drive; never hangs forever
  (bounded status spins). No IRQ14 yet.
- **`kernel/src/fs.rs`** — tiny read-only "GOATFS" at fixed LBA 2048 (must
  stay past the boot sector + kernel; Makefile enforces kernel sector count
  `< 2048`). Superblock + one directory sector + file data.
- **`scripts/build-goatfs.py`** — packs `build/goatfs.img`; the Makefile
  `dd`s it into `build/disk.img` at that LBA. Ships a `hello.txt` whose
  contents are also a string constant in `fs.rs` so the boot self-test can
  check them.
- **Shell `cat <file>`** — looks up a name in the mounted directory and
  prints the bytes (UTF-8 text).

## Suggested order from here

1. ~~**A real kernel-side disk driver.**~~ Done for reads (ATA PIO primary
   master). Still missing: writes, secondary bus / slave, IRQ-driven
   transfers, anything beyond 28-bit LBA.
2. ~~**A simple on-disk format** good enough for `cat`.~~ Done (GOATFS). A
   well-documented format like FAT16/FAT32 is the natural next step if
   host-side tooling or multi-file write support starts to matter - FAT
   images are easy to inspect/populate from a desktop OS.
3. **A minimal VFS abstraction** (`open` / `read` / `write` / `close`-style
   trait) before wiring in a second filesystem type, so the rest of the
   kernel doesn't end up hard-coded to GOATFS.
4. A **ramdisk/initrd** was considered as an intermediate before ATA existed;
   with ATA reads working it is optional. Still useful if you want a
   filesystem that never depends on a real drive.

## Conventions to follow

- Disk I/O errors must be handled, not `unwrap()`'d - see the
  defensive-driver pattern in `qemu-testing-and-verification` and
  `drivers`. ATA already returns `AtaError::{NotPresent,Timeout,...}`;
  keep that shape for writes.
- Keep `FS_BASE_LBA` (kernel) and the Makefile `seek=` in lockstep. If the
  kernel ever grows past ~1 MiB of sectors, move the FS base and update
  both.
- If/when this needs testing against real files, prefer building small,
  purpose-made images via `scripts/build-goatfs.py` (or a successor) over
  hand-crafting bytes in Rust test code.
- Spot-check disk I/O under v86 as well as QEMU - see
  `web-demo-packaging`. BIOS extended LBA reads hang under v86; kernel-side
  ATA port I/O is the supported path.
