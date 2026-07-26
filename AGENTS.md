# AGENTS.md

## Cursor Cloud specific instructions

GOATos is a from-scratch bare-metal x86_64 OS kernel written in Rust. It is a
Cargo workspace with two crates: `kernel/` (the `#![no_std]` OS, built for the
`x86_64-unknown-none` target) and the root `goatos` runner (`build.rs` +
`src/main.rs`), which builds bootable BIOS/UEFI disk images and boots the BIOS
image in QEMU. See `README.md` for the full build/run flow.

### Environment notes

- The Rust nightly toolchain, the `x86_64-unknown-none` target, and the
  `llvm-tools-preview`/`rust-src` components are pinned in
  `rust-toolchain.toml` and auto-installed by rustup on first build.
- **QEMU (`qemu-system-x86_64`) is a required system dependency** that is NOT
  managed by Cargo. It is installed during environment setup
  (`apt-get install -y qemu-system-x86`) and baked into the VM snapshot, so it
  is not part of the update script. If `cargo run` fails with "failed to start
  `qemu-system-x86_64`", reinstall QEMU.

### Build / lint / run

- Build: `cargo build` (see `README.md`).
- Lint: `cargo clippy` (clean). Note: `cargo fmt --check` currently reports a
  pre-existing formatting diff in `src/main.rs`; do not "fix" it unless asked.
- Run: `cargo run` (see `README.md`). This boots the kernel in QEMU headlessly
  (`-display none`) and forwards COM1 serial to stdio.

### Running / testing gotchas

- **The kernel never exits.** After printing its boot message it halts in an
  infinite `hlt` loop, so `cargo run` (and QEMU) run forever. To capture output
  non-interactively, wrap it in a timeout, e.g.
  `timeout 20 cargo run 2>&1`, or stop it with `Ctrl-C`.
- A successful boot prints, over serial:
  `GOATos booted successfully!` followed by
  `bootloader reported N memory region(s)`. The lines prefixed with `INFO :`
  come from the bootloader, not the kernel.
- There is no framebuffer/display output yet, so QEMU is run headlessly; do not
  expect a GUI window.
