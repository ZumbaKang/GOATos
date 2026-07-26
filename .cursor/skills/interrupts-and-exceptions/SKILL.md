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
  ring-0 data `0x10`), replacing the throwaway one `boot.asm` builds. Still in
  `.rodata`, so installing a TSS descriptor at runtime needs it made writable
  first.
- `kernel/src/idt.rs` - the 256-entry IDT, `set_handler` /
  `set_handler_with_error_code`, and `lidt`/`sidt`. Handlers take the stack
  frame **by value** (`extern "x86-interrupt" fn(StackFrame)`); a pointer
  parameter is materialised *from* the frame instead of pointing at it.
- `kernel/src/exceptions.rs` - handlers for divide error (0), invalid opcode
  (6), and general protection fault (13). Each prints the vector, faulting
  `eip`/`cs`/`eflags` (and the error code, for 13) to VGA *and* serial, then
  halts. No recovery, by design.
- Debug-only `trigger-*` cargo features raise one of those exceptions on
  purpose, e.g. `make run KERNEL_FEATURES=trigger-divide-error` - the way to
  re-verify handler changes against a real fault instead of by inspection.

## What is still missing

Double fault (8) with its own TSS stack, PIC remapping, `sti`, and a default
handler for unregistered vectors - in that order (see `ROADMAP.md` phase 1).
Note that until a double-fault handler exists, a fault *inside* a handler
still triple-faults into a silent reboot.

## Suggested order for the rest

1. **Double fault (8).** Needs a dedicated stack via a TSS, so that a fault
   raised *by* a stack overflow can still be reported; a double fault taken on
   the overflowed stack faults again and triple-faults the machine.
2. **Page fault (14)**, once paging exists - the error code and `CR2` say
   nearly everything about the bug that caused it.
3. **PIC setup** to actually receive hardware interrupts (timer, keyboard).
   Remap the legacy PIC's vectors so they don't collide with the CPU
   exception vectors (0-31); that collision is a classic, well documented
   gotcha.
4. Once that's in place, a **timer (PIT) interrupt handler** and a
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
