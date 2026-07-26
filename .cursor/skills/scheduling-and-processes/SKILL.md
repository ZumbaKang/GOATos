---
name: scheduling-and-processes
description: Guidance for adding multitasking to GOATos (kernel tasks/threads, context switching, and eventually a scheduler) - not yet built, and blocked on memory management and interrupts. Use this when starting work on tasks, threads, or a scheduler.
---

# Scheduling and processes (not yet implemented)

GOATos is currently single-threaded: `kernel_main` runs once and halts
forever. This is a later-stage feature - it depends on both
`memory-management` (a stack per task needs to come from somewhere) and
`interrupts-and-exceptions` (preemptive scheduling needs a timer
interrupt).

## Suggested order of implementation

1. **Cooperative tasks first.** Before attempting preemptive multitasking,
   get simple cooperative "green threads" working: each task gets its own
   stack (allocated once a heap exists), and a hand-written context switch
   (saving/restoring registers, including `esp`, to switch between tasks)
   that only happens when a task explicitly yields. This is much easier to
   get right and debug than jumping straight to preemption.
2. **A minimal scheduler.** Even a simple round-robin ready-queue is
   enough to start with; sophistication (priorities, fairness) can come
   later.
3. **Preemption.** Once the PIT timer interrupt exists (see
   `interrupts-and-exceptions`), have its handler trigger a context switch
   periodically instead of relying on cooperative yields.
4. **Ring 3 / userspace**, if GOATos ever wants to run untrusted or
   isolated code, is a much bigger step from here (new GDT segments,
   `iretq`-based ring transitions, syscalls) - don't reach for it until
   kernel-mode tasking is solid.

## Conventions to follow

- Keep this in its own module (e.g. `kernel/src/task/`), not mixed into
  `main.rs`.
- Context-switch assembly, if needed, should follow the same pattern as
  `kernel/src/entry.s`: a small `.s` file included via
  `global_asm!(include_str!(...), options(att_syntax))`, with a clear
  comment explaining the calling convention/register save order.
- A bug in the scheduler is exactly the kind of thing that benefits most
  from the double-fault/exception handling work in
  `interrupts-and-exceptions` being done first - a broken context switch
  usually manifests as a silent crash otherwise.
