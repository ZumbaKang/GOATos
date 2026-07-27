---
name: qemu-testing-and-verification
description: How to build, boot, and verify GOATos changes headlessly (no display) using QEMU - serial output conventions, capturing a real VGA screenshot via the QEMU monitor for visual proof, and defensive-driver patterns that keep the kernel debuggable even when a peripheral misbehaves. Use this whenever you change kernel code and need to confirm it actually boots.
---

# QEMU testing and verification

This sandbox (and most CI environments) has no display, so "does it boot"
has to be verified without a human watching a window.

## Standard build + boot

```bash
make run           # builds the kernel + disk image, boots headlessly,
                    # forwards the kernel's serial (COM1) output to this
                    # terminal
make run-display    # same, but with a graphical QEMU window (only useful
                    # if you actually have a display)
```

`make run` never returns on its own (the kernel halts in an infinite loop
after printing, so QEMU keeps the "VM" up) - run it under `timeout N` when
scripting/automating a check, and confirm the expected serial line was
printed before the timeout kills it.

## Capturing a real screenshot (VGA proof), not just serial text

Serial output only proves the kernel's logic ran - it says nothing about
what actually renders on screen, which matters a lot for GOATos since VGA
text output is the point (see `web-demo-packaging`). To get a real image:

```bash
qemu-system-i386 -drive file=build/disk.img,format=raw -display none \
  -serial file:/tmp/serial.log \
  -monitor unix:/tmp/mon.sock,server,nowait &
QPID=$!
sleep 1.5
python3 -c "
import socket, time
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect('/tmp/mon.sock'); time.sleep(0.3); s.recv(4096)
s.sendall(b'screendump /tmp/scr.ppm\n'); time.sleep(0.3); s.recv(4096)
s.close()
"
kill $QPID
```

Then convert the PPM to something viewable: `convert /tmp/scr.ppm
/tmp/scr.png` (ImageMagick). This is genuinely useful for confirming VGA
driver changes (colors, layout, scrolling) actually look right, not just
that the code compiles and "should" work.

## Where QEMU's CPU emulation is *not* faithful

Worth knowing before designing a mechanism around a CPU feature and finding out
it can't be tested:

- **Segment limits are not enforced** by QEMU's TCG emulation. Verified
  directly: a write through a deliberately bounded expand-down data segment,
  well outside its limit, silently succeeded instead of raising #GP. So
  segmentation cannot be used to bound the kernel stack - the unmapped
  guard page below it (roadmap 2.6, now in place) is what catches overflows.
- v86 has its own, different gaps - notably no double-fault escalation. See
  `web-demo-packaging` for the list, and check the browser console before
  blaming the kernel.

Both directions of this matter: a mechanism that "works" under QEMU may be
unverifiable in v86, and vice versa, so a feature that has to hold on real
hardware is best proved under both.

## Defensive driver design (important pattern, learned the hard way)

Early on, the serial driver's `.expect()`-on-failure pattern caused a
**silent, total hang with zero output** when the serial port's self-test
failed under a different emulator (v86) - see `web-demo-packaging` for the
full story. The fix, and the pattern to follow for any *new* peripheral
driver:

- A driver failing to initialize (or a write failing) must never panic or
  hang the whole kernel. Store `Option<Device>`, and have the
  print/write path silently no-op if it's `None`.
- `kernel_main` does VGA output **before** anything else, specifically so
  VGA - the primary, "this has to work" output surface - is never gated on
  a secondary/debug peripheral (currently: serial) succeeding.
- The panic handler itself must not depend on anything that could itself
  be in a broken state, or a panic can recursively fail silently instead of
  printing anything.

When adding a new driver (keyboard, timer, disk, etc.), keep this same
shape: best-effort init, graceful no-op on failure, and make sure a failure
in one driver can't block another's output.
