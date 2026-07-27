//! Single-CPU synchronisation helpers.
//!
//! GOATos has no SMP and no preemption beyond interrupts, so the interesting
//! race is always the same one: an interrupt handler that needs a lock the
//! interrupted code already holds. A plain spin lock deadlocks there; the
//! types here are what keep diagnostic printing (and anything else a handler
//! must be able to do) from hanging the machine.

use core::arch::asm;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

/// EFLAGS bit 9 - the interrupt flag. Saved/restored around critical sections
/// so a handler that itself takes an [`IrqMutex`] does not accidentally
/// re-enable interrupts before `iret`.
const EFLAGS_IF: u32 = 1 << 9;

/// A mutex that never spins forever against an interrupt handler on the same
/// CPU.
///
/// The fast path masks maskable interrupts, takes the lock, runs the
/// closure, and restores IF - so a hardware IRQ cannot observe the lock as
/// held and deadlock trying to take it. The slow path covers everything that
/// ignores IF (software `int`, CPU exceptions): if the lock is already held,
/// the closure still runs. That is sound on a single CPU because the holder
/// is the interrupted context and cannot run until the handler returns; the
/// two critical sections are nested, not concurrent. Cursor/state may tear if
/// both sides mutate the same data, but a torn diagnostic beats a silent hang.
pub struct IrqMutex<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: exclusive `&mut T` is only handed out while this CPU either holds
// the lock or has interrupted the holder; `T: Send` is enough for a static.
unsafe impl<T: Send> Sync for IrqMutex<T> {}
unsafe impl<T: Send> Send for IrqMutex<T> {}

impl<T> IrqMutex<T> {
    /// Creates a mutex wrapping `value`.
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    /// Runs `f` with exclusive access to the protected value.
    ///
    /// Always runs `f`, even when the lock is already held (interrupt
    /// re-entry on this CPU). See the type-level docs for why that is safe
    /// here and why a spinning [`spin::Mutex`] is not.
    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let saved_flags = read_eflags();
        // SAFETY: masking IF for a short critical section is the standard
        // single-CPU way to keep a hardware IRQ from nesting into it; the
        // matching restore below puts IF back exactly as we found it.
        unsafe { asm!("cli", options(att_syntax, nomem, nostack)) };

        let acquired = self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok();

        // SAFETY: either we just acquired the lock, or the holder is the
        // context we interrupted on this CPU and is suspended for the rest of
        // this call. In both cases we have exclusive access until `f` returns.
        let result = f(unsafe { &mut *self.value.get() });

        if acquired {
            self.locked.store(false, Ordering::Release);
        }

        if saved_flags & EFLAGS_IF != 0 {
            // SAFETY: IF was set when we entered, so restoring it cannot grant
            // more interruptibility than the caller already had.
            unsafe { asm!("sti", options(att_syntax, nomem, nostack)) };
        }

        result
    }
}

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
