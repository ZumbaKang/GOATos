---
name: scheduling-and-processes
description: Guidance for adding multitasking to GOATos (kernel tasks/threads, context switching, and eventually a scheduler). Cooperative switching exists; a ready-queue scheduler and preemption do not. Use this when starting work on tasks, threads, or a scheduler.
---

# Scheduling and processes

Cooperative kernel tasks exist (`kernel/src/task/`): each task has its own
stack and switches only on an explicit `task::yield_now`. There is not yet
a ready-queue scheduler (roadmap 4.4) or timer preemption.

## What is already in place (roadmap 4.3)

- `kernel/src/task/switch.s` — `context_switch` saves/restores ebx/esi/edi/ebp
  and swaps `esp`; `task_trampoline` boots a freshly spawned stack into its
  `fn() -> !` entry (with a SysV 16-byte align fixup before `call`).
- `kernel/src/task/mod.rs` — fixed table of up to `MAX_TASKS` (4). Task 0 is
  the bootstrap context (shell) on the kernel stack from `entry.s`; spawned
  tasks get an 8 KiB heap stack. Yield walks the table for the next live slot.
- Boot wiring in `main.rs`: shell task + demo counter task. Shell yields then
  `hlt`s; the PIT wake is what gives the demo task CPU once per tick burst.
  Interleaving proof: serial `PIT: tick` / `Task: demo counter N` pairs
  (grepped by `scripts/ci-test.sh`).

## Suggested order from here

1. ~~**Cooperative tasks first.**~~ Done (4.3).
2. **A minimal scheduler (4.4).** A real ready-queue that decides which task
   runs next after a yield; prove 3+ tasks round-robin fairly. The current
   "next occupied slot" walk is enough for two tasks but is not that queue.
3. **Preemption.** Have the PIT handler trigger a context switch periodically
   instead of relying only on cooperative yields. Mind `IrqMutex` / lock
   state: do not switch away while holding a spin lock.
4. **Ring 3 / userspace**, if GOATos ever wants to run untrusted or
   isolated code, is a much bigger step from here (new GDT segments,
   `iret`-based ring transitions, syscalls) - don't reach for it until
   kernel-mode tasking is solid.

## Conventions to follow

- Keep this in its own module (`kernel/src/task/`), not mixed into `main.rs`.
- Context-switch assembly follows the same pattern as `kernel/src/entry.s`:
  a small `.s` file included via
  `global_asm!(include_str!(...), options(att_syntax))`, with a clear
  comment explaining the calling convention/register save order.
- Cast function items through `as *const ()` before turning them into
  integers (`clippy::function_casts_as_integer`).
- Release any scheduler lock *before* calling `context_switch`; mask IRQs
  across the switch itself so a handler cannot observe half-updated state.
- A bug in the scheduler is exactly the kind of thing that benefits most
  from the double-fault/exception handling work in
  `interrupts-and-exceptions` being done first - a broken context switch
  usually manifests as a silent crash otherwise.
