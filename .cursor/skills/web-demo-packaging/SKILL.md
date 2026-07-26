---
name: web-demo-packaging
description: How the browser-based GOATos demo works (v86, web/index.html, scripts/build-web-demo.sh), how to rebuild and test it locally with a headless browser, and specific v86 compatibility gotchas that were discovered the hard way. Use this when changing web/, scripts/build-web-demo.sh, the boot process, or anything that affects what shows up on screen.
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

To test locally:

```bash
./scripts/build-web-demo.sh
python3 -m http.server -d _site 8080
# open http://localhost:8080
```

## Testing headlessly (no display), with a real browser

A system Chrome + `puppeteer` (via `npm install puppeteer`, pointed at
`executablePath: "/usr/local/bin/google-chrome"` with `--no-sandbox`) can
drive the page and either read the live text overlay
(`document.querySelector("#screen_container div").innerText`) or take a
real `page.screenshot()`. This was essential for debugging boot issues that
only manifested under v86, not QEMU - see below.

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
4. **A UART loopback self-test (`Uart16550::test_loopback()`) hung under
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
