---
name: web-demo-packaging
description: How the browser-based GOATos demo works (v86, web/index.html, web/gui.html, scripts/build-web-demo.sh), how to rebuild and test it locally with a headless browser, and specific v86 compatibility gotchas that were discovered the hard way. Use this when changing web/, scripts/build-web-demo.sh, the boot process, or anything that affects what shows up on screen.
---

# Web demo packaging (v86)

GOATos boots live in a browser via [v86](https://github.com/copy/v86), an
x86 emulator compiled to WebAssembly. `scripts/build-web-demo.sh`:

1. runs `make disk` to build `build/disk.img` (the exact same image QEMU
   boots for local testing - one boot path, no divergence)
2. fetches the v86 JS/WASM runtime via `npm install v86` into a scratch
   directory (not committed to the repo - it's a large binary blob and
   changes independently of GOATos)
3. downloads `seabios.bin`/`vgabios.bin` straight from the v86 GitHub repo
   (the npm package doesn't ship them)
4. assembles everything into `$OUT_DIR` (`_site/` by default), ready to be
   served as a static site (this is what CI publishes to GitHub Pages)

## Pages

| Path | File | Purpose |
|------|------|---------|
| `/` | `web/index.html` | Hub with links |
| `/gui.html` | `web/gui.html` | **Primary GUI test page**: CSS-scaled Mode 13h canvas (320×200 → 3×) plus a live serial (COM1) log panel |

Since roadmap 5.1 the kernel runs in VGA Mode 13h. v86 switches from the
text `<div>` to the `<canvas>` inside `#screen_container` automatically.
`gui.html` is what you open on GitHub Pages to see framebuffer work; the
serial panel mirrors what `make run` / CI grep for.

To test locally:

```bash
./scripts/build-web-demo.sh
python3 -m http.server -d _site 8080
# open http://localhost:8080/gui.html
```

## Testing headlessly (no display), with a real browser

A system Chrome + `puppeteer` (via `npm install puppeteer`, pointed at
`executablePath: "/usr/local/bin/google-chrome"` with `--no-sandbox`) can
drive the page and either read the serial panel
(`#serial_log` on `gui.html`), sample the graphics canvas, or take a real
`page.screenshot()`. This was essential for debugging boot issues that
only manifested under v86, not QEMU - see below.

For GUI track verification, prefer loading `/gui.html`, waiting until
`#serial_log` contains the framebuffer banner (e.g. `Framebuffer: VGA mode`),
then screenshotting the scaled canvas region.

## v86 compatibility gotchas discovered while building this

These cost real debugging time; don't re-discover them:

1. **v86's tiny/non-standard-sized disk images fail to boot at all**
   (`Boot failed: could not read the boot disk`). Pad the disk image to a
   comfortable size (GOATos uses 10MiB via `truncate` in the `Makefile`) -
   BIOS geometry detection gets confused by extremely small "hard disks".
2. **`hda` (raw disk) booted further than `cdrom` (El Torito/ISO) for the
   same GRUB-built image** when this project still used GRUB - CD-ROM/ATAPI
   emulation in v86 is less battle-tested than plain IDE hard-disk
   emulation. (Moot now that GOATos uses its own hand-written bootloader
   and a raw `hda` image directly, but keep it in mind if CD-ROM boot ever
   comes up again.)
3. **The BIOS extended/LBA disk read service (`INT 13h/AH=42h`) hung
   indefinitely under v86**, with zero error output, even though the exact
   same code path worked fine in real QEMU. Classic CHS reads
   (`INT 13h/AH=02h`, see `bootloader-and-linking`) do not have this
   problem. If a disk read seems to silently hang only under v86, suspect
   the extended-read BIOS service first.
4. **v86 does not implement escalating a failed exception delivery into a
   double fault.** When the kernel deliberately makes exception delivery
   fail (`KERNEL_FEATURES=trigger-double-fault`), v86 aborts the whole
   emulator - `panicked at src/rust/cpu/cpu.rs: Unimplemented: #GP handler`,
   visible only in the browser console, with the screen simply frozen. QEMU
   raises #8 correctly. v86 *does* implement 32-bit hardware task switching,
   so `trigger-double-fault-gate` (a plain `int $8` through the same task
   gate) is the way to exercise a task-gate handler in the browser. General
   lesson: when the v86 screen freezes with no kernel output, read the
   browser console before suspecting the kernel - an "Unimplemented" panic
   there means v86, not GOATos, gave up.
5. **A UART loopback self-test (`Uart16550::test_loopback()`) hung under
   v86.** This was the trickiest bug: the kernel would boot, run
   correctly, and then freeze with *zero* output (not even VGA) because
   the serial driver's init panicked, and the panic handler's own attempt
   to log over the (uninitialized) serial port panicked again. The fix was
   twofold: don't call `test_loopback()`/`check_connected()` at all (they
   aren't needed just to transmit), and make driver failures non-fatal
   (see `qemu-testing-and-verification`).

## Debugging a "boots in QEMU but not in v86" issue

This is the failure mode you're most likely to hit. A reliable process,
used repeatedly to find the bugs above:

1. Reproduce with the headless-browser + screenshot setup above.
2. Add a cheap checkpoint: either a BIOS teletype print
   (`mov ah, 0x0e` / `int 0x10`, real mode only) or a direct VGA buffer
   write (`movw $0x2f50, 0xb8000`, works in both real and protected mode,
   no BIOS calls needed - useful once you're past the point where BIOS
   interrupts are still valid).
3. Bisect forward through the boot sequence (real mode -> disk read -> A20
   -> GDT -> protected mode entry -> jump to kernel -> `kernel_main`) one
   checkpoint at a time until you find exactly where v86's screen output
   stops updating.
4. Remember `boot/boot.asm` has a strict 512-byte budget - temporary debug
   code there often needs *other* code trimmed to fit, and should always
   be removed again once the bug is found (this repo does not keep
   `%ifdef DEBUG_*` scaffolding around; strip it out in the same change
   that fixes the bug).
