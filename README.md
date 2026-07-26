# GOATos

GOATos is a from-scratch operating system, built iteratively (with the help
of an AI coding agent) starting from nothing but a bootloader handoff and a
"hello world" over the serial port.

## Status

- [x] Boots on x86_64 via BIOS (through the [`bootloader`](https://github.com/rust-osdev/bootloader) crate) and prints a boot confirmation over the serial port (COM1).
- [ ] Interrupts (GDT / IDT), exception handling
- [ ] Physical & virtual memory management (paging, a heap allocator)
- [ ] Keyboard input
- [ ] A minimal shell
- [ ] Framebuffer text output
- [ ] ...and much more, one step at a time.

## How it works

- `kernel/` is the actual operating system kernel: a `#![no_std]`, `#![no_main]`
  Rust binary compiled for the bare-metal `x86_64-unknown-none` target. It
  has no dependency on any OS - it *is* the OS.
- The root crate (`Cargo.toml`, `build.rs`, `src/main.rs`) is a small **runner**:
  its `build.rs` uses the `bootloader` crate to turn the compiled kernel into
  bootable BIOS/UEFI disk images, and `src/main.rs` boots the BIOS image in
  QEMU.
- The kernel currently just brings up a serial port driver, prints
  `GOATos booted successfully!`, and then halts forever. That's it - but it's
  a real kernel, booted by a real (emulated) BIOS, on a real bootloader. Every
  future feature builds on top of this.

## Prerequisites

- **Rust nightly** with the `x86_64-unknown-none` target and the
  `llvm-tools-preview` component. This is all pinned in
  [`rust-toolchain.toml`](rust-toolchain.toml) - `rustup`/`cargo` will install
  it automatically the first time you build.
- **QEMU**, specifically `qemu-system-x86_64`. On Debian/Ubuntu:

  ```bash
  sudo apt-get update && sudo apt-get install -y qemu-system-x86
  ```

## Build & run

From the repository root:

```bash
cargo run
```

This will:

1. Compile the `kernel` crate for `x86_64-unknown-none`.
2. Use the `bootloader` crate (in `build.rs`) to embed it into a bootable
   BIOS disk image (and, for future use, a UEFI image too).
3. Launch that disk image in QEMU headlessly (`-display none`), forwarding
   the kernel's serial output (COM1) to your terminal via `-serial stdio`.

You should see output similar to:

```
GOATos booted successfully!
physical memory offset: Some(...)
```

The kernel halts in an infinite loop after printing, so QEMU will keep
running until you stop it (e.g. `Ctrl-C`, or run
`cargo run -- -no-shutdown` and close the (absent, in headless mode) window /
send QEMU a `quit` via its monitor).

## Next steps

Roughly in order of dependency:

1. **GDT + IDT + exception handlers** so a bug in the kernel produces a
   readable panic/double-fault message instead of a silent reboot.
2. **Paging & a heap allocator** so the kernel can use dynamic memory
   (`Vec`, `Box`, etc. via `alloc`).
3. **Keyboard + timer interrupts** for interactive input and preemption.
4. **A simple in-kernel shell / task scheduler.**
5. **Framebuffer text rendering**, using the framebuffer the `bootloader`
   crate already exposes in `BootInfo`, so output doesn't depend on a serial
   console.
