//! Fixed-size input event queue.
//!
//! Decouples "a key arrived" (IRQ1 handler) from "something acts on it"
//! (`kernel_main`'s idle loop, and eventually the shell's line editor). The
//! producer is always the keyboard interrupt; the consumer is always normal
//! kernel code. Both sides take [`crate::sync::IrqMutex`], so a push never
//! races a pop on this single-CPU kernel.
//!
//! Capacity is fixed and small - enough for a burst of typing between idle
//! wakes, with no heap involved. A full queue drops the newest event rather
//! than blocking the IRQ handler (which would leave the keyboard line stuck).

use crate::sync::IrqMutex;

/// How many events the queue can hold. Comfortably above normal typing speed
/// between `hlt` wakes; a held key's autorepeat still fits without pressure.
pub const CAPACITY: usize = 64;

/// A translated key, ready for a consumer that does not speak scancodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEvent {
    /// A printable character (letters, digits, space), Shift already applied.
    Char(char),
    /// Enter / Return.
    Enter,
    /// Backspace.
    Backspace,
}

struct Queue {
    buf: [KeyEvent; CAPACITY],
    /// Next slot to write.
    head: usize,
    /// Next slot to read.
    tail: usize,
    /// How many events are currently stored.
    len: usize,
    /// Events dropped because the queue was full (diagnostic).
    dropped: u32,
}

impl Queue {
    const fn new() -> Self {
        Self {
            // Dummy fill; `len == 0` means none of these are live.
            buf: [KeyEvent::Char('\0'); CAPACITY],
            head: 0,
            tail: 0,
            len: 0,
            dropped: 0,
        }
    }

    fn push(&mut self, event: KeyEvent) -> bool {
        if self.len == CAPACITY {
            self.dropped = self.dropped.saturating_add(1);
            return false;
        }
        self.buf[self.head] = event;
        self.head = (self.head + 1) % CAPACITY;
        self.len += 1;
        true
    }

    fn pop(&mut self) -> Option<KeyEvent> {
        if self.len == 0 {
            return None;
        }
        let event = self.buf[self.tail];
        self.tail = (self.tail + 1) % CAPACITY;
        self.len -= 1;
        Some(event)
    }

    fn len(&self) -> usize {
        self.len
    }

    fn dropped(&self) -> u32 {
        self.dropped
    }
}

static QUEUE: IrqMutex<Queue> = IrqMutex::new(Queue::new());

/// Pushes `event` onto the queue. Returns `false` if the queue was full and
/// the event was dropped - the IRQ handler must not block either way.
pub fn push(event: KeyEvent) -> bool {
    QUEUE.with(|q| q.push(event))
}

/// Removes and returns the oldest event, or `None` if the queue is empty.
pub fn pop() -> Option<KeyEvent> {
    QUEUE.with(|q| q.pop())
}

/// How many events are waiting. Useful for tests; ordinary code can just
/// [`pop`] until it gets `None`.
pub fn len() -> usize {
    QUEUE.with(|q| q.len())
}

/// How many events have been dropped because the queue was full since boot.
pub fn dropped() -> u32 {
    QUEUE.with(|q| q.dropped())
}
