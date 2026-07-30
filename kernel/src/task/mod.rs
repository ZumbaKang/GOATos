//! Round-robin cooperative scheduler (roadmap 4.4).
//!
//! Each task owns a stack and a saved stack pointer. Switching happens only
//! when a task calls [`yield_now`] - there is no timer preemption yet. Ready
//! tasks sit in a FIFO queue: a yield parks the current task at the tail and
//! runs the head, so 3+ tasks take equal turns.
//!
//! The bootstrap context that reaches [`init`] becomes task 0 and keeps the
//! kernel stack from `entry.s`; anything [`spawn`]ed gets a fresh heap-backed
//! stack and is enqueued ready.

use core::arch::{asm, global_asm};
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use alloc::boxed::Box;

use spin::Mutex;

global_asm!(include_str!("switch.s"), options(att_syntax));

/// Bytes of stack handed to each spawned task. Plenty for the shell / demo
/// loops and small enough that a couple of them fit comfortably in the 1 MiB
/// heap.
pub const STACK_SIZE: usize = 8 * 1024;

/// Slots in the task table. Shell + two demos for 4.4 leave one spare.
pub const MAX_TASKS: usize = 4;

/// Saved stack pointer for a parked task. The stack itself holds the
/// callee-saved registers and return address; see `switch.s`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Context {
    esp: u32,
}

struct Task {
    context: Context,
    /// Keeps the heap allocation alive for spawned tasks. Unused for the
    /// bootstrap task, which runs on the kernel stack from `entry.s`.
    _stack: Option<Box<[u8]>>,
}

/// Fixed task table plus a FIFO ready queue. The running task is *not* in
/// the queue; [`yield_now`] pushes it onto the tail and pops the head.
struct Scheduler {
    tasks: [Option<Task>; MAX_TASKS],
    ready: [usize; MAX_TASKS],
    ready_head: usize,
    ready_len: usize,
    current: usize,
}

impl Scheduler {
    const fn new() -> Self {
        Self {
            tasks: [None, None, None, None],
            ready: [0; MAX_TASKS],
            ready_head: 0,
            ready_len: 0,
            current: 0,
        }
    }

    fn enqueue_ready(&mut self, id: usize) {
        // A full queue would mean every slot is ready *and* one is running,
        // which cannot happen with MAX_TASKS slots.
        debug_assert!(self.ready_len < MAX_TASKS);
        let tail = (self.ready_head + self.ready_len) % MAX_TASKS;
        self.ready[tail] = id;
        self.ready_len += 1;
    }

    fn dequeue_ready(&mut self) -> Option<usize> {
        if self.ready_len == 0 {
            return None;
        }
        let id = self.ready[self.ready_head];
        self.ready_head = (self.ready_head + 1) % MAX_TASKS;
        self.ready_len -= 1;
        Some(id)
    }
}

static SCHEDULER: Mutex<Scheduler> = Mutex::new(Scheduler::new());

/// Number of tasks that have been created (bootstrap + spawns). Reported in
/// the boot banner.
static TASK_COUNT: AtomicUsize = AtomicUsize::new(0);

/// How many times each task has been switched *to*. Used to prove the
/// ready-queue round-robins fairly.
static TURNS: [AtomicU32; MAX_TASKS] = [
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
];

extern "C" {
    fn context_switch(old_esp: *mut u32, new_esp: *mut u32);
    fn task_trampoline();
}

/// Registers the calling context as task 0. Must run once, on the kernel
/// stack, before any [`spawn`] / [`yield_now`]. Task 0 starts running and is
/// not placed on the ready queue.
pub fn init() {
    let mut sched = SCHEDULER.lock();
    sched.tasks[0] = Some(Task {
        // Filled in on the first yield; nothing reads it before then.
        context: Context { esp: 0 },
        _stack: None,
    });
    sched.current = 0;
    sched.ready_head = 0;
    sched.ready_len = 0;
    TASK_COUNT.store(1, Ordering::Relaxed);
}

/// How many tasks exist (running or ready).
pub fn count() -> usize {
    TASK_COUNT.load(Ordering::Relaxed)
}

/// Id of the task that is currently running.
pub fn current() -> usize {
    SCHEDULER.lock().current
}

/// How many times `id` has been scheduled in via a yield (not counting the
/// bootstrap task's initial ownership of the CPU).
pub fn turns(id: usize) -> u32 {
    if id >= MAX_TASKS {
        return 0;
    }
    TURNS[id].load(Ordering::Relaxed)
}

/// Creates a new task that will start at `entry` the first time it is
/// scheduled. The new task is enqueued ready. Returns the task id, or
/// `None` if the table is full.
pub fn spawn(entry: fn() -> !) -> Option<usize> {
    let mut stack = alloc::vec![0u8; STACK_SIZE].into_boxed_slice();
    // Align the high end so the trampoline can re-align for a SysV call after
    // popping the entry pointer (see `task_trampoline` in `switch.s`).
    let stack_top = (stack.as_mut_ptr() as usize + stack.len()) & !0xf;

    // Stack grows down. Highest addresses first:
    //   [entry fn pointer]   — popped by `task_trampoline` after the first ret
    //   [task_trampoline]    — context_switch's ret lands here
    //   [ebp, edi, esi, ebx] — zeroed callee-saved slots
    let mut esp = stack_top;
    esp -= 4;
    unsafe {
        (esp as *mut u32).write(entry as *const () as usize as u32);
    }
    esp -= 4;
    unsafe {
        (esp as *mut u32).write(task_trampoline as *const () as usize as u32);
    }
    for _ in 0..4 {
        esp -= 4;
        unsafe {
            (esp as *mut u32).write(0);
        }
    }

    let mut sched = SCHEDULER.lock();
    let id = sched.tasks.iter().position(|slot| slot.is_none())?;
    sched.tasks[id] = Some(Task {
        context: Context { esp: esp as u32 },
        _stack: Some(stack),
    });
    sched.enqueue_ready(id);
    TASK_COUNT.fetch_add(1, Ordering::Relaxed);
    Some(id)
}

/// Parks the current task at the tail of the ready queue and runs the head.
/// If we are the only task, this is a no-op.
pub fn yield_now() {
    let (old_esp, new_esp) = {
        let mut sched = SCHEDULER.lock();
        let current = sched.current;
        sched.enqueue_ready(current);
        // After the push there is always at least one ready entry (us).
        let next = sched
            .dequeue_ready()
            .expect("ready queue nonempty after enqueue");
        if next == current {
            // Sole task: we enqueued and immediately dequeued ourselves.
            return;
        }

        // SAFETY: both indices were just checked against occupied slots, and
        // the Task values outlive the switch (they sit in this 'static table).
        let old_esp = &mut sched.tasks[current].as_mut().unwrap().context.esp as *mut u32;
        let new_esp = &mut sched.tasks[next].as_mut().unwrap().context.esp as *mut u32;
        sched.current = next;
        TURNS[next].fetch_add(1, Ordering::Relaxed);
        (old_esp, new_esp)
    };

    // Release the scheduler lock *before* switching (skill convention). Mask
    // IRQs across the switch so a handler cannot observe half-updated state.
    // `context_switch` returns on this stack later, still inside this
    // function, so the matching `sti` runs in the resumed task.
    let flags = read_eflags();
    // SAFETY: brief cli around a pure register/stack shuffle; IF restored
    // below from the saved flags.
    unsafe {
        asm!("cli", options(att_syntax, nomem, nostack));
        context_switch(old_esp, new_esp);
    }
    if flags & EFLAGS_IF != 0 {
        // SAFETY: IF was set on entry, so restoring it cannot grant more
        // interruptibility than the caller already had.
        unsafe {
            asm!("sti", options(att_syntax, nomem, nostack));
        }
    }
}

const EFLAGS_IF: u32 = 1 << 9;

fn read_eflags() -> u32 {
    let eflags: u32;
    // SAFETY: push/pop of EFLAGS touches only the current stack and a register.
    unsafe {
        asm!(
            "pushfl",
            "popl {flags}",
            flags = out(reg) eflags,
            options(att_syntax),
        );
    }
    eflags
}
