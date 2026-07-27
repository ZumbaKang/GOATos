//! Minimal shell line editor (roadmap 4.1).
//!
//! Sits on top of the Phase 3 input queue: typed characters append to a
//! fixed-size line buffer and echo as they arrive, backspace removes the
//! last character, and enter submits the line so the whole thing can be
//! echoed back. Built-in commands (roadmap 4.2) will replace that echo
//! with a dispatcher; for now the submitted line is the whole product.

use crate::input::{self, KeyEvent};
use crate::{serial_print, serial_println, vga, vga_print, vga_println};

/// Max characters in one input line. Leaves room for a short prompt on an
/// 80-column VGA row without wrapping mid-edit.
pub const LINE_CAPACITY: usize = 72;

/// Prompt shown at the start of each editable line.
const PROMPT: &str = "> ";

/// Fixed-size editable input line.
pub struct LineEditor {
    buf: [u8; LINE_CAPACITY],
    len: usize,
}

impl Default for LineEditor {
    fn default() -> Self {
        Self::new()
    }
}

impl LineEditor {
    pub const fn new() -> Self {
        Self {
            buf: [0; LINE_CAPACITY],
            len: 0,
        }
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    fn as_str(&self) -> &str {
        // Only ASCII printable bytes are ever pushed, so this is always
        // valid UTF-8.
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }

    fn push_char(&mut self, c: char) -> bool {
        if self.len >= LINE_CAPACITY {
            return false;
        }
        // The keyboard driver only emits ASCII printables; reject anything
        // else so the buffer stays UTF-8-safe without a wider encoding path.
        if !matches!(c, ' '..='~') {
            return false;
        }
        self.buf[self.len] = c as u8;
        self.len += 1;
        true
    }

    fn pop_char(&mut self) -> bool {
        if self.len == 0 {
            return false;
        }
        self.len -= 1;
        true
    }

    /// Applies one key event. Returns `true` if Enter submitted a line
    /// (useful for tests; ordinary code can ignore it).
    pub fn handle_event(&mut self, event: KeyEvent) -> bool {
        match event {
            KeyEvent::Char(c) => {
                if self.push_char(c) {
                    vga_print!("{}", c);
                    serial_print!("{}", c);
                }
                false
            }
            KeyEvent::Backspace => {
                if self.pop_char() {
                    vga::backspace();
                    serial_print!("\x08 \x08");
                }
                false
            }
            KeyEvent::Enter => {
                vga_println!();
                serial_println!();
                // Echo the whole submitted line back - the "Done when" for 4.1.
                let line = self.as_str();
                vga_println!("{}", line);
                serial_println!("{}", line);
                self.clear();
                print_prompt();
                true
            }
        }
    }
}

/// Prints the initial prompt. Call once after the input queue is ready.
pub fn init() {
    print_prompt();
}

fn print_prompt() {
    vga_print!("{}", PROMPT);
    serial_print!("{}", PROMPT);
}

/// Pops every pending input event into `editor`.
pub fn drain_input(editor: &mut LineEditor) {
    while let Some(event) = input::pop() {
        editor.handle_event(event);
    }
}
