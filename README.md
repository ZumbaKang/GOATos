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
- [x] **Interrupts and exception handling**: a kernel-owned GDT/IDT, readable
      crash reports for the CPU exceptions a kernel bug is likely to trip (a
      double fault on a stack of its own, via a TSS and task gate), both 8259
      PICs remapped clear of the exception vectors, and interrupts enabled with
      a catch-all for any vector nothing owns yet.
- [x] **Physical & virtual memory management**: BIOS E820 memory map, a
      physical frame allocator, identity-mapped paging, a kernel heap with
      `#[global_allocator]`/`alloc`, and an unmapped guard page below the
      kernel stack so overflow double-faults instead of corrupting memory.
- [x] Keyboard input (PIT timer + PS/2 keyboard on IRQ0/IRQ1, input event
      queue decoupling the IRQ handler from consumers)
- [x] **A minimal shell**: line editor on the keyboard queue, built-ins
      (`help`/`clear`/`echo`/`about`/`cat`), a cooperative round-robin
      scheduler (FIFO ready queue, explicit yield) with 3+ fair tasks, and
      a tiny on-disk filesystem (ATA PIO + GOATFS) so `cat` reads real files
      from the disk image.
- [ ] Graphics mode + a basic GUI
- [ ] ...and much more, one step at a time.

Each of those is a big milestone made of many small steps - see
[**ROADMAP.md**](ROADMAP.md) for the breakdown of specific, individually
shippable tasks that lead up to each one.

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

## Continuous integration & automated development

Every PR is built, linted (`cargo clippy`), and boot-tested headlessly in
QEMU by [`.github/workflows/ci.yml`](.github/workflows/ci.yml) (the same
check as `make test` locally). PRs from a `cursor/*` branch are taken out of
draft and then merged automatically (squash + delete branch) once that
passes - PRs on other branch names, and any PR from a fork, are left for
manual review.

That auto-merge is what makes it possible for GOATos to build itself out
with minimal supervision: a [Cursor Automation](https://cursor.com/automations)
can pick up the next task from [`ROADMAP.md`](ROADMAP.md), implement it,
and open a PR; if CI passes, it merges on its own, and the next run picks
up the next task. See
[`.cursor/skills/roadmap-automation/`](.cursor/skills/roadmap-automation/SKILL.md)
for the exact procedure (and for how to set the Automation itself up - that
part happens in the Cursor dashboard, not a repo file).

Before merging, CI also waits for and respects the
[`Cursor Bugbot`](https://cursor.com/docs/bugbot) check, if Bugbot is
enabled for this repo - a PR it flags never auto-merges out from under a
finding. See
[`.cursor/skills/bugbot-and-code-review/`](.cursor/skills/bugbot-and-code-review/SKILL.md)
for how that gate works and how to enable Bugbot + its Autofix feature
(also dashboard-only) so flagged issues get fixed automatically too.

## Skills

This repo uses [Cursor Agent Skills](https://cursor.com/docs/skills) to
capture domain knowledge for an AI agent iterating on GOATos over many
sessions. See [`.cursor/skills/`](.cursor/skills/) - each skill covers one
subsystem (the bootloader, drivers, the eventual memory manager, the web
demo pipeline, etc.), including hard-won debugging lessons (e.g. specific
v86 compatibility gotchas) that are worth preserving rather than
re-discovering.

## Next steps

Roughly in order of dependency: interrupts/exceptions, memory management,
keyboard/timer input, a minimal shell, then a graphics mode + basic GUI.
See [**ROADMAP.md**](ROADMAP.md) for that broken down into small, specific
tasks with "done when" criteria, and `.cursor/skills/` for the deeper
technical guidance behind each phase.

Automations (scheduled/triggered agent runs building out the above) are a
planned future addition once there's more of the OS built to give them
useful work to do - see [Cursor Automations](https://cursor.com/automations).
