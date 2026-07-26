# GOATos

GOATos is a from-scratch operating system, built iteratively (with the help
of an AI coding agent) starting from nothing but a hand-written bootloader
and a "hello world" on screen.

## Status

- [x] Boots on x86 (32-bit protected mode) via a **hand-written bootloader**
      (no GRUB, no third-party bootloader crate) and displays a boot
      confirmation on the VGA text screen, plus a copy over the serial port
      for headless testing.
- [x] **Boots live in a browser** via [v86](https://github.com/copy/v86) -
      the exact same disk image QEMU boots for local testing. See
      [Web demo](#web-demo) below.
- [ ] Interrupts (GDT / IDT), exception handling
- [ ] Physical & virtual memory management (paging, a heap allocator)
- [ ] Keyboard input
- [ ] A minimal shell
- [ ] Graphics mode + a basic GUI
- [ ] ...and much more, one step at a time.

## How it works

- **`boot/boot.asm`** is the entire bootloader: a 512-byte, hand-written
  MBR boot sector. It loads the kernel off the disk (BIOS CHS reads),
  enables the A20 line, sets up a flat GDT, switches the CPU to 32-bit
  protected mode, and jumps straight into the kernel - no GRUB, no
  Multiboot, no BIOS assistance beyond what every PC (real or emulated)
  provides for free.
- **`kernel/`** is the actual operating system kernel: a `#![no_std]`,
  `#![no_main]` Rust binary built for a custom bare-metal 32-bit x86 target
  (`kernel/i686-goatos.json`) and linked as a flat binary
  (`kernel/linker.ld`) - it has no dependency on any OS; it *is* the OS.
- **`Makefile`** wires the two together: it builds the kernel, computes how
  many disk sectors it needs, assembles the boot sector with that count
  baked in, and concatenates everything into one bootable raw disk image.
- The kernel currently brings up a VGA text-mode driver and a serial port
  driver, prints a boot confirmation to both, and halts forever. That's it
  - but it's a real kernel, booted by a real (emulated or physical) BIOS,
  by a bootloader written from scratch for this project. Every future
  feature builds on top of this.

## Prerequisites

- **Rust nightly**, with the `rust-src` and `llvm-tools-preview`
  components. This is pinned in [`rust-toolchain.toml`](rust-toolchain.toml)
  - `rustup`/`cargo` install it automatically on first build.
- **`nasm`** (assembles `boot/boot.asm`) and **binutils' `objcopy`**
  (flattens the linked kernel ELF into a raw binary).
- **QEMU**, specifically `qemu-system-i386`.

On Debian/Ubuntu:

```bash
sudo apt-get update && sudo apt-get install -y nasm binutils qemu-system-x86
```

## Build & run

From the repository root:

```bash
make run
```

This will:

1. Compile the `kernel` crate for the custom 32-bit target.
2. Flatten it into a raw binary and assemble `boot/boot.asm` around its
   exact size.
3. Concatenate boot sector + kernel into `build/disk.img`.
4. Boot that image in QEMU headlessly (`-display none`), forwarding the
   kernel's serial output (COM1) to your terminal via `-serial stdio`.

You should see:

```
GOATos: loading kernel...
GOATos booted successfully! (32-bit, from a hand-written bootloader)
```

Use `make run-display` instead if you have a display and want to see the
actual VGA text output (the primary "hello world" surface); `make run` only
shows the serial copy, since this is meant to also work headlessly (e.g. in
CI or a cloud sandbox). The kernel halts in an infinite loop after
printing, so QEMU keeps running until you stop it (`Ctrl-C`).

Other useful targets: `make disk` (just build the disk image), `make clean`.

## Web demo

Because the kernel boots via a plain, hand-written bootloader with no
64-bit/long-mode requirement, the exact same disk image that QEMU boots
also boots in [v86](https://github.com/copy/v86), a pure-JS/WebAssembly x86
emulator - meaning GOATos can be published as a static website with no
backend server at all.

```bash
./scripts/build-web-demo.sh   # builds the disk image + assembles a static
                               # site into _site/ (fetches the v86 runtime
                               # via npm; nothing large is committed to git)
python3 -m http.server -d _site 8080
# open http://localhost:8080
```

`web/index.html` is the demo page; `.github/workflows/deploy-pages.yml`
rebuilds and republishes it to GitHub Pages automatically on every push to
`main`.

## Skills

This repo uses [Cursor Agent Skills](https://cursor.com/docs/skills) to
capture domain knowledge for an AI agent iterating on GOATos over many
sessions. See [`.cursor/skills/`](.cursor/skills/) - each skill covers one
subsystem (the bootloader, drivers, the eventual memory manager, the web
demo pipeline, etc.), including hard-won debugging lessons (e.g. specific
v86 compatibility gotchas) that are worth preserving rather than
re-discovering.

## Next steps

Roughly in order of dependency (see `.cursor/skills/` for detailed guidance
on each):

1. **GDT + IDT + exception handlers** so a bug in the kernel produces a
   readable panic/double-fault message instead of a silent freeze.
2. **Paging & a heap allocator** so the kernel can use dynamic memory
   (`Vec`, `Box`, etc. via `alloc`).
3. **Keyboard + timer interrupts** for interactive input and preemption.
4. **A simple in-kernel shell / task scheduler.**
5. **A graphics mode + basic GUI**, building on the same "write to a
   buffer the BIOS/emulator already renders" idea the VGA text driver
   already uses.

Automations (scheduled/triggered agent runs building out the above) are a
planned future addition once there's more of the OS built to give them
useful work to do - see [Cursor Automations](https://cursor.com/automations).
