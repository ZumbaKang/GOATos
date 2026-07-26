---
name: interrupts-and-exceptions
description: Guidance for setting up the GDT/IDT and CPU exception handlers in GOATos - not yet built. Use this when a kernel bug needs a real fault/panic message instead of a silent hang or reboot, or before adding keyboard/timer interrupt support.
---

# Interrupts and exceptions (not yet implemented)

Right now, GOATos runs with interrupts disabled the whole time (`cli` in
`boot/boot.asm`, never re-enabled) and has no IDT of its own - it's still
relying on whatever the CPU's default/BIOS-leftover state happens to be.
Any bug that triggers a CPU exception (divide-by-zero, page fault once
paging exists, general protection fault, etc.) currently has no defined,
useful behavior. This is the next thing worth building after (or alongside)
`memory-management`, since it turns "the kernel just froze, good luck" into
a readable diagnostic - hugely valuable for iterating quickly.

## Suggested order of implementation

1. **A proper GDT.** `boot/boot.asm` already sets up a minimal flat GDT to
   get into protected mode; the kernel should build its own (in Rust, not
   assembly) once it wants more than one segment type (e.g. for a future
   ring 3 / userspace).
2. **An IDT with handlers for the CPU exceptions that matter first**:
   divide error (0), invalid opcode (6), general protection fault (13),
   page fault (14, once paging exists), and double fault (8) - double
   fault especially, since without a working handler for it, a fault
   *while handling a fault* triple-faults the CPU and silently reboots the
   VM, which is a nasty thing to debug blind. A double-fault handler needs
   its own dedicated stack (a "task state segment" IST entry) so a
   stack-overflow-induced double fault can still be handled.
3. **PIC (or APIC) setup** to actually receive hardware interrupts (timer,
   keyboard), remapping the legacy PIC's interrupt vectors so they don't
   collide with the CPU exception vectors (0-31) is a classic, well
   documented gotcha.
4. Once that's in place, a **timer (PIT) interrupt handler** and a
   **keyboard interrupt handler** are natural next drivers (see the
   `drivers` skill).

## Conventions to follow

- Exception/interrupt handlers should print via VGA first (it's the
  primary output surface - see `qemu-testing-and-verification`), and via
  serial as a bonus, exactly like the existing panic handler in
  `kernel/src/main.rs`.
- A crash from an unhandled exception should be at least as informative as
  the existing `#[panic_handler]` - include the exception vector, error
  code (where applicable), and faulting address/instruction pointer.
- The [OSDev wiki](https://osdev.wiki/) pages "Interrupt Descriptor Table",
  "Exceptions", and "8259 PIC" are the standard references. The
  `x86_64` crate's IDT/GDT abstractions are a good reference for API shape
  even though this kernel is 32-bit (see `bootloader-and-linking` for why);
  a similarly-scoped 32-bit-only crate or hand-written structs work fine.
