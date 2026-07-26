---
name: interrupts-and-exceptions
description: How GOATos's GDT/IDT and CPU exception handlers work (kernel/src/gdt.rs, idt.rs, exceptions.rs), and what is still missing. Use this when a kernel bug needs a real fault message instead of a silent hang or reboot, or before adding keyboard/timer interrupt support.
---

# Interrupts and exceptions

GOATos still runs with interrupts disabled the whole time (`cli` in
`boot/boot.asm`, never re-enabled), but it does own its descriptor tables and
reports the exceptions an ordinary kernel bug is most likely to trip.

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
  collide with the exception vectors), with every IRQ line then masked.
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
- Both TSSes carry a `cr3` that the CPU loads on the switch. It is 0 today
  because paging is off; **when paging lands it must become the kernel's real
  page directory.**

### Provoking a real double fault

A kernel cannot raise #8 directly - it is what the CPU reports when it fails to
deliver *another* exception. `trigger-double-fault` manufactures exactly that:
`idt::clear_handler(0)` then a divide by zero, so delivery of #DE raises #GP,
and two contributory exceptions in a row *are* the definition of a double
fault.

The textbook cause, a stack overflow, cannot be provoked yet: with no paging
there is no guard page below the kernel stack, and segment limits (the only
other way to bound a stack) **are not enforced by QEMU's TCG emulation** -
verified directly, an out-of-range write through a deliberately bounded
expand-down segment silently succeeds. So an overflow corrupts memory rather
than faulting; roadmap task 2.6 closes that once paging exists.

**v86 does not implement the escalation at all** - it aborts the whole emulator
with `panicked at src/rust/cpu/cpu.rs: Unimplemented: #GP handler`. It *does*
implement 32-bit hardware task switching, so `trigger-double-fault-gate`
(a plain `int $8` through the same gate) is what proves the switch and the
private stack in the browser demo, with output identical to QEMU's.

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
and 6 enabled). A masked-but-latched IRQ shows up there as `irr=01`, which is
the timer waiting for permission it will not get until roadmap task 1.6.

## What is still missing

`sti`, and a default handler for unregistered vectors - in that order (see
`ROADMAP.md` phase 1).

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
