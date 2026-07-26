---
name: drivers
description: Conventions for writing new hardware drivers in GOATos (keyboard, PIT timer, disk/ATA beyond the boot sector, etc.), based on the existing vga.rs/serial.rs drivers. Use this whenever adding a new peripheral driver.
---

# Writing a new driver

GOATos currently has two drivers to use as templates:

- `kernel/src/vga.rs` - the VGA text-mode (0xB8000) writer. The primary,
  load-bearing output surface (see `qemu-testing-and-verification` for why
  it comes first in `kernel_main`).
- `kernel/src/serial.rs` - the COM1 UART writer. A best-effort, secondary
  debugging aid.

Both follow the same shape, which new drivers should copy:

1. A private struct wrapping the raw hardware state, behind a
   `static X: spin::Mutex<...>` (no heap allocation available yet - see
   `memory-management` - so this has to be a `const`-constructible static).
2. A public `init()` function, callable exactly once, that's **infallible
   from the caller's perspective**: internal failures are swallowed/logged,
   never `panic!`/`unwrap()`/`expect()`'d, per the defensive-driver lesson
   in `qemu-testing-and-verification`. A driver failing to initialize must
   never be able to hang or crash the whole kernel.
3. A `#[doc(hidden)] pub fn _print(...)` plus a pair of
   `<name>_print!`/`<name>_println!` macros (see the bottom of `vga.rs`/
   `serial.rs`) if the driver produces text output. Keep this pattern for
   consistency even if it feels repetitive - it makes call sites in
   `kernel_main` and elsewhere predictable.
4. Liberal `# Safety` doc comments on any `unsafe fn`, explaining exactly
   what the caller must guarantee (matches the existing style).

## Where new drivers plug in

- **Keyboard** and **PIT (timer)**: both need working interrupts first -
  see the `interrupts-and-exceptions` skill. Until then, a keyboard driver
  can only be *polled* (checking the PS/2 controller's status port
  directly), which works but is wasteful and doesn't scale to real input
  handling.
- **Disk (beyond the boot sector)**: `boot/boot.asm` already contains a
  minimal, working CHS-based disk-read routine (see
  `bootloader-and-linking`) - written in assembly, for boot-time use only.
  A kernel-side disk driver (for a future filesystem) should be written in
  Rust, live in `kernel/src/`, and should very deliberately use the same
  CHS approach rather than the BIOS's LBA "extended read" service, which
  was found to hang under the v86 browser emulator (see
  `web-demo-packaging`) - though note that once the kernel is running,
  BIOS calls aren't available at all anymore (no real/protected-mode
  callback path was set up), so a real disk driver has to talk to the
  ATA/IDE controller's I/O ports directly, not via `INT 13h`.

## Testing

Always verify a new driver via `make run` (serial output) and, if it
produces any visible output, a VGA screendump - see
`qemu-testing-and-verification`. If the driver could plausibly behave
differently under v86 (anything doing I/O port access, disk access, or
timing-sensitive polling loops), also test via the web demo pipeline - see
`web-demo-packaging`.
