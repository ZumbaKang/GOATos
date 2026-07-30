---
name: bootloader-and-linking
description: How GOATos boots (the hand-written boot sector in boot/boot.asm, the custom i686 Rust target, and the flat-binary linker script), and what to check before changing any of it. Use this when touching boot/boot.asm, kernel/linker.ld, kernel/i686-goatos.json, kernel/.cargo/config.toml, or kernel/src/entry.s.
---

# Bootloader and linking

GOATos has no GRUB, no Multiboot, and no third-party bootloader crate. The
whole boot chain is:

1. BIOS loads sector 0 (`boot/boot.asm`, exactly 512 bytes) to `0x7C00` and
   jumps to it in 16-bit real mode.
2. `boot.asm` queries the BIOS memory map (`INT 15h, EAX=0xE820`) and leaves
   it at `0x500` for the kernel to read once the BIOS is out of reach (see
   `memory-management`), queries disk geometry (`INT 13h/AH=08h`), reads the
   kernel's remaining sectors off the same disk one sector at a time via classic
   CHS `INT 13h/AH=02h` reads into memory starting at physical `0x10000`,
   enables the A20 line, switches VGA to Mode 13h (`INT 10h` / `AH=00`,
   `AL=13h` - last chance before the BIOS is unreachable; see
   `graphics-and-gui`), builds a flat GDT, and switches to 32-bit
   protected mode.
3. It then does a **near jump straight to `0x10000`** - the load address -
   with no ELF parsing and no symbol awareness. Whatever bytes are first in
   the loaded image *are* the entry point.
4. `kernel/src/entry.s`'s `_start32` (placed via its own `.entry` section,
   pinned first in `kernel/linker.ld`) sets up a stack and calls
   `kernel_main` in `kernel/src/main.rs`.

## Hard-won lessons (read before changing the boot path)

- **The kernel is a flat binary, not an ELF the loader understands.**
  `Makefile` runs `objcopy -O binary` on the linked ELF to strip it down to
  a raw blob. This means whatever code is physically first in the output
  file is what gets executed first - there is no `_start` symbol lookup at
  boot time. `linker.ld` enforces this with `KEEP(*(.entry)) ` before
  `*(.text .text.*)`, and `entry.s` puts `_start32` in its own
  `.entry, "ax", @progbits` section (an explicit flags argument is
  required - unrecognized section names default to *non-allocatable*
  without it, which silently drops the section from the loaded image).
- **`boot/boot.asm` must stay within exactly 512 bytes.** Assembling it
  with `nasm` will fail loudly (`TIMES value N is negative`) if it doesn't.
  When adding anything to it (even temporary debug prints), expect to trim
  something else to make room. Mode 13h set (`mov ax,0x0013` / `int 0x10`)
  is five bytes but often lands free thanks to the GDT `align 8` slack
  shifting; measure rather than assume:
  `nasm -f bin boot/boot.asm -D KERNEL_SECTORS=1 -o /tmp/b.bin`, then find the
  last non-zero byte before the `0xaa55` signature. The next thing that needs
  real space in real mode (VBE mode info blocks, etc.) probably needs a
  second-stage loader instead of more MBR bytes.
- **Use CHS (`INT 13h/AH=02h`) disk reads, not the LBA "extended read"
  (`AH=42h`).** This was tested and found to hang under the v86 browser
  emulator (see `web-demo-packaging` for how that was diagnosed) despite
  working fine in real QEMU. CHS is the older, more universally-implemented
  BIOS disk service.
- **A single real-mode segment:offset transfer buffer can't safely span
  more than 64KiB.** The kernel is already well past 64KB. `load_kernel` in
  `boot.asm` reads one sector (512B) at a time and advances the destination
  *segment* by 32 (paragraphs) after each read - don't "optimize" this into
  a multi-sector-per-call read without re-deriving the segment math.
- **`KERNEL_SECTORS` is computed by the Makefile from the kernel binary's
  actual size** and passed to `nasm` via `-D KERNEL_SECTORS=N`. Never
  hardcode a sector count in `boot.asm`.
- The kernel's load address (`0x10000`), its custom target
  (`kernel/i686-goatos.json`), and its linker script all have to agree on
  where code/data end up. If you change one, check the other two.

## Toolchain notes

- The custom target requires cargo's `json-target-spec` unstable feature
  (`kernel/.cargo/config.toml`'s `[unstable]` table) - this is a fairly
  recent (2026) requirement after custom JSON targets were destabilized on
  stable Rust; without it you'll see
  `` `.json` target specs require -Zjson-target-spec ``.
- `build-std = ["core", "compiler_builtins"]` is required since there's no
  prebuilt `core` for this custom target; it needs the `rust-src` component
  (already pinned in the repo's `rust-toolchain.toml`).
- Field types in target JSON files are stricter than most older tutorials
  show: `target-pointer-width` and `target-c-int-width` must be JSON
  numbers, not strings, and `rustc-abi` must be `"softfloat"`, not
  `"x86-softfloat"`, on current nightly.
