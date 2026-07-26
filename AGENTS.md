# AGENTS.md

## Cursor Cloud specific instructions

GOATos is a from-scratch 32-bit x86 OS kernel written in Rust, booted by a
hand-written MBR bootloader (`boot/boot.asm`) — no GRUB, no bootloader crate.
The `Makefile` orchestrates the build: it compiles the `kernel/` crate for the
custom bare-metal target `kernel/i686-goatos.json`, flattens the ELF to a raw
binary with `objcopy`, assembles the boot sector with `nasm` (baking in the
kernel's sector count), and concatenates them into `build/disk.img`. See
`README.md` and `.cursor/skills/` for full details.

### Environment notes

- The Rust nightly toolchain and the `rust-src`/`llvm-tools-preview`
  components are pinned in `rust-toolchain.toml` and auto-installed by rustup.
  The kernel builds with `build-std` and a JSON target spec (see
  `kernel/.cargo/config.toml`), so no prebuilt `std`/target is needed.
- **System dependencies (NOT managed by Cargo), installed during environment
  setup and baked into the VM snapshot — so they are not in the update
  script:**
  - `nasm` — assembles `boot/boot.asm`.
  - `binutils` (`objcopy`) — flattens the kernel ELF into a raw binary.
  - `qemu-system-i386` (from `qemu-system-x86`) — boots the disk image.
  - The web demo additionally uses `node`/`npm` (fetch v86 runtime), `curl`
    (fetch v86 BIOS blobs), and `python3` (serve the site) — all preinstalled.

### Build / lint / run

- Build kernel + disk image: `make disk` (or `make` for just the kernel). See
  the `Makefile` for all targets.
- Lint: `cd kernel && cargo clippy` (clean).
- Run: `make run` — boots headlessly in QEMU, forwarding COM1 serial to stdio.
  `make run-display` opens a graphical window (needs a display).

### Running / testing gotchas

- **The kernel never exits.** After printing it halts in an infinite loop, so
  `make run` (and QEMU) run forever. When scripting a check, wrap it in a
  timeout, e.g. `timeout 15 make run`, and confirm the expected line printed.
- Successful boot prints over serial:
  `GOATos booted successfully! (32-bit, from a hand-written bootloader)`.
  VGA text output is the *primary* surface (serial is a headless copy); to
  prove VGA rendering, capture a screenshot via the QEMU monitor as documented
  in `.cursor/skills/qemu-testing-and-verification/SKILL.md`. Converting the
  resulting `.ppm` needs Pillow/ImageMagick (Pillow is available via `pip`;
  ImageMagick/netpbm are not preinstalled).
- Web demo: `./scripts/build-web-demo.sh` builds `build/disk.img` and assembles
  a static site in `_site/` (fetches v86 via npm + BIOS blobs via curl — needs
  network). Serve with `python3 -m http.server -d _site 8080`; the exact same
  `disk.img` QEMU boots also boots in-browser via v86.
