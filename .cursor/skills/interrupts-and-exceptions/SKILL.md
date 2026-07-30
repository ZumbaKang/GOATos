---
name: interrupts-and-exceptions
description: How GOATos's GDT/IDT and CPU exception handlers work (kernel/src/gdt.rs, idt.rs, exceptions.rs), and what is still missing. Use this when a kernel bug needs a real fault message instead of a silent hang or reboot, or before adding keyboard/timer interrupt support.
---

# Interrupts and exceptions

GOATos owns its descriptor tables, reports the exceptions an ordinary kernel
bug is most likely to trip, and runs with interrupts enabled - every vector
goes somewhere, though no IRQ line is unmasked yet, so nothing is actually
delivered until a driver asks for one.

## What exists now

- `kernel/src/gdt.rs` - a kernel-owned flat GDT (null / ring-0 code `0x08` /
  ring-0 data `0x10`) plus descriptors for the two TSSes below, replacing the
  throwaway table `boot.asm` builds. It lives in an `UnsafeCell` (not
  `.rodata`) because the TSS descriptors are built at runtime and the CPU
  writes their busy bits itself.
- `kernel/src/idt.rs` - the 256-entry IDT, `set_handler` /
  `set_handler_with_error_code` / `set_task_gate` / `clear_handler`, and
  `lidt`/`sidt`. Handlers take the stack frame **by value**
  (`extern "x86-interrupt" fn(StackFrame)`); a pointer parameter is
  materialised *from* the frame instead of pointing at it.
- `kernel/src/tss.rs` - two Task State Segments and a private 4 KiB stack, the
  machinery behind the double-fault handler (see below).
- `kernel/src/exceptions.rs` - handlers for divide error (0), invalid opcode
  (6), double fault (8), and general protection fault (13). Each prints the
  vector, faulting `eip`/`cs`/`eflags` (and the error code, for 13) to VGA
  *and* serial, then halts. No recovery, by design.
- `kernel/src/pic.rs` - the two cascaded 8259 PICs, reprogrammed so IRQ0-15
  raise vectors 32-47 instead of the BIOS's real-mode 8-15/0x70-0x77 (which
  collide with the exception vectors), with every IRQ line then masked. Also
  `end_of_interrupt`, which acknowledges a serviced IRQ (and recognises a
  spurious one, which must *not* be acknowledged).
- `kernel/src/interrupts.rs` - `sti`, plus the catch-all that makes it safe:
  every vector `exceptions::init` didn't claim gets an entry point that reports
  what arrived. See below.
- Debug-only `trigger-*` cargo features raise one of those exceptions on
  purpose, e.g. `make run KERNEL_FEATURES=trigger-divide-error` - the way to
  re-verify handler changes against a real fault instead of by inspection.

## The double fault, and why it is a *task* gate

32-bit x86 has no Interrupt Stack Table (that's an x86-64 addition). An
interrupt taken while already in ring 0 keeps using the stack it interrupted,
and a TSS's `ss0`/`esp0` are consulted *only* on a privilege-level change - so
they do not help a ring-0 kernel at all. The one architectural way to make a
handler start on a known-good stack is a **task gate**: the CPU does a full
hardware task switch, saving the interrupted registers into the TSS named by
the task register and loading a complete new set (`esp` included) from the TSS
named by the gate.

Hence two TSSes: `MAIN_TSS` exists purely to be written to (a task switch with
nowhere to save the outgoing state faults), and `DOUBLE_FAULT_TSS` is a
prepared register image pointing at `exceptions::double_fault_entry`. Things
that follow from this and are easy to get wrong:

- `tss::init` must run **before** `gdt::init`, which builds the descriptors and
  runs `ltr`. `gdt::loaded().tr` reads the task register back; `tr=0x00` means a
  double fault would triple-fault, which `scripts/ci-test.sh` now checks for.
- The double-fault entry point is a plain `extern "C" fn() -> !`, *not* an
  `extern "x86-interrupt"` handler: it is a task entry point, so there is no
  interrupt frame on the stack (read the interrupted state from the main TSS
  via `tss::interrupted_state()`) and no `iret` to emit.
- Both TSSes carry a `cr3` that the CPU loads on the switch.
  `tss::set_page_directory` writes the real directory once paging is on;
  leaving it at 0 would triple-fault the handler.

### Provoking a real double fault

A kernel cannot raise #8 directly - it is what the CPU reports when it fails to
deliver *another* exception. Two features cover the two shapes:

- `trigger-double-fault` manufactures a "fault during delivery":
  `idt::clear_handler(0)` then a divide by zero, so delivery of #DE raises #GP,
  and two contributory exceptions in a row *are* the definition of a double
  fault. Does not depend on paging.
- `trigger-stack-overflow` is the textbook cause: infinite recursion grows
  `esp` into the unmapped guard page below the kernel stack (roadmap 2.6).
  Pushing the #PF frame fails, and the CPU escalates to the vector-8 task gate.
  Segment limits cannot substitute for that guard - QEMU's TCG (and v86) do
  not enforce them.

**v86 does not implement the escalation at all** - it aborts the whole emulator
with `panicked at src/rust/cpu/cpu.rs: Unimplemented: #GP handler`. It *does*
implement 32-bit hardware task switching, so `trigger-double-fault-gate`
(a plain `int $8` through the same gate) is what proves the switch and the
private stack in the browser demo, with output identical to QEMU's. Do not
expect `trigger-stack-overflow` / `trigger-double-fault` to print cleanly
under v86.

## The PIC remap, and why the mask matters

The 8259 initialization sequence (ICW1-ICW4 to ports 0x20/0x21 and 0xa0/0xa1)
*clears* the interrupt mask register as a side effect, i.e. it leaves every IRQ
line enabled. `pic::init` therefore writes 0xff back to both masks, and that is
load-bearing rather than tidy: the PIT is ticking from the moment the machine
powers on, so the first `sti` with an unmasked line would dispatch vector 32
before anything has registered a handler for it. Each driver unmasks its own
line once it has one.

The programmed vector base cannot be read back - ICW2 is write-only - so the
boot banner reports the range the kernel *programmed* next to the masks it
actually read from the hardware. To check the remap from outside the kernel,
ask QEMU: `-monitor unix:/tmp/mon.sock,server,nowait`, then `info pic` prints
`irq_base=20`/`irq_base=28` and `imr=ff` for the two controllers (before the
remap: `irq_base=08`/`irq_base=70`, with `imr=b8` - the BIOS leaves IRQ0, 1, 2
and 6 enabled). A masked-but-latched IRQ shows up there as `irr=01`: the timer,
waiting for permission it will not get until something unmasks its line.

## The catch-all, and why enabling interrupts needed one

`idt::init` leaves a vector it was not asked about "not present", and taking a
not-present vector raises a #GP *whose own delivery* counts as a second
exception - so before `sti`, a single stray interrupt would have escalated to a
double fault. "Interrupts are on" and "every vector goes somewhere" therefore
had to land together: `interrupts::init` fills the 252 vectors
`exceptions::init` doesn't claim, and only then does `interrupts::enable` run
`sti`.

Things about it that are easy to get wrong:

- An `extern "x86-interrupt"` handler is told nothing about which vector it was
  entered for, so the only way for the report to name it is one entry point per
  vector: `unhandled::<VECTOR>` is monomorphised 256 times (in rows of 16, via
  a macro, since a const-generic argument can be a literal expression but not
  an expression over another const parameter). That costs ~26 KB of image -
  worth it for reports that say `#14 page fault` instead of `#14`.
- The shape has to match the vector: 8, 10, 11, 12, 13, 14, 17, 21, 29 and 30
  are entered with an error code already pushed. A handler with the wrong shape
  reads the error code where it expects `eip`.
- Disposition differs by vector, and both directions matter. An unhandled
  *exception* must halt: `iret` would re-execute the faulting instruction,
  fault again, and print again forever. A stray *IRQ* (or an `int n` to an
  unused vector) must resume - halting the machine over one is a worse outcome
  than a diagnostic - but it has to be acknowledged first, or the controller
  keeps that line and every lower-priority one blocked.
- A vector reports only the first time it fires. A line unmasked with no driver
  behind it arrives continuously, and the thousandth copy of a report buries
  the first one, which is the one with context in it.
- `pic::end_of_interrupt` deliberately takes no lock, unlike the rest of that
  module: it runs in interrupt context, where blocking on a lock the
  interrupted code holds would hang the kernel. It doesn't need one - the ports
  and the bytes written are constants.

Both `trigger-unhandled-interrupt` (resume path, and the report-once
suppression) and `trigger-unhandled-exception` (halt path) exercise it, and
`trigger-spurious-irq` takes vector 39 to reach the phantom-IRQ branch. v86
implements the OCW3 in-service read that branch depends on, and produces output
identical to QEMU's.

## What is still missing

Nothing in phase 1. The next interrupt-adjacent work is in `ROADMAP.md`
phases 2 and 3 (see below).

## Suggested order for the rest

1. **Page fault (14)**, once paging exists - the error code and `CR2` say
   nearly everything about the bug that caused it.
2. Once that's in place, a **timer (PIT) interrupt handler** and a
   **keyboard interrupt handler** are natural next drivers (see the
   `drivers` skill).

## Conventions to follow

- Exception/interrupt handlers should print via VGA first (it's the
  primary output surface - see `qemu-testing-and-verification`), and via
  serial as a bonus; `exceptions.rs`'s `diag_println!` does both in that
  order.
- A crash report should be at least as informative as the existing
  `#[panic_handler]` - include the exception vector, error code (where
  applicable), and faulting address/instruction pointer.
- Prove a new handler against a real fault, in QEMU *and* v86, before
  believing it: add a `trigger-*` feature the way `exceptions.rs` does. The
  reported `eip` should match `objdump -d --start-address=...` of the
  instruction that was supposed to fault.
- The [OSDev wiki](https://osdev.wiki/) pages "Interrupt Descriptor Table",
  "Exceptions", and "8259 PIC" are the standard references. The
  `x86_64` crate's IDT/GDT abstractions are a good reference for API shape
  even though this kernel is 32-bit (see `bootloader-and-linking` for why);
  a similarly-scoped 32-bit-only crate or hand-written structs work fine.
