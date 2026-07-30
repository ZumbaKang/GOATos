//! Minimal interactive shell (roadmap 4.1 / 4.2 / 4.5).
//!
//! Sits on top of the Phase 3 input queue: typed characters append to a
//! fixed-size line buffer and echo as they arrive, backspace removes the
//! last character, and enter submits the line to a small built-in command
//! dispatcher (`help` / `clear` / `echo` / `about` / `cat`).

use crate::fs::{self, FsError};
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
                run_line(self.as_str());
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

/// Dispatches a submitted line to a built-in, or reports an unknown command.
fn run_line(line: &str) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }

    let (cmd, rest) = split_command(line);
    match cmd {
        "help" => cmd_help(),
        "clear" => cmd_clear(),
        "echo" => cmd_echo(rest),
        "about" => cmd_about(),
        "cat" => cmd_cat(rest),
        other => {
            vga_println!("unknown command: {}", other);
            serial_println!("unknown command: {}", other);
        }
    }
}

/// Splits `line` into the first whitespace-delimited word and the remainder
/// (with leading whitespace on the remainder stripped).
fn split_command(line: &str) -> (&str, &str) {
    let trimmed = line.trim_start();
    match trimmed.find(char::is_whitespace) {
        Some(idx) => {
            let (cmd, rest) = trimmed.split_at(idx);
            (cmd, rest.trim_start())
        }
        None => (trimmed, ""),
    }
}

fn cmd_help() {
    vga_println!("Built-in commands:");
    vga_println!("  help   - list commands");
    vga_println!("  clear  - clear the VGA screen");
    vga_println!("  echo   - print arguments");
    vga_println!("  about  - about GOATos");
    vga_println!("  cat    - print a file from disk");
    serial_println!("Built-in commands:");
    serial_println!("  help   - list commands");
    serial_println!("  clear  - clear the VGA screen");
    serial_println!("  echo   - print arguments");
    serial_println!("  about  - about GOATos");
    serial_println!("  cat    - print a file from disk");
}

fn cmd_clear() {
    vga::clear_screen();
    // Serial has no clear; mark the boundary so headless logs stay readable.
    serial_println!("(screen cleared)");
}

fn cmd_echo(args: &str) {
    vga_println!("{}", args);
    serial_println!("{}", args);
}

fn cmd_about() {
    vga_println!("GOATos - a from-scratch 32-bit x86 OS in Rust");
    vga_println!("Booted via a hand-written MBR bootloader.");
    serial_println!("GOATos - a from-scratch 32-bit x86 OS in Rust");
    serial_println!("Booted via a hand-written MBR bootloader.");
}

/// Reads `path` from the mounted GOATFS image and prints its bytes.
fn cmd_cat(args: &str) {
    let path = args.trim();
    if path.is_empty() {
        vga_println!("usage: cat <file>");
        serial_println!("usage: cat <file>");
        return;
    }
    // One path argument only - no flags, no globbing.
    if path.chars().any(char::is_whitespace) {
        vga_println!("usage: cat <file>");
        serial_println!("usage: cat <file>");
        return;
    }

    let mut buf = [0u8; fs::MAX_FILE_SIZE];
    match fs::read_file(path, &mut buf) {
        Ok(n) => {
            // Files are treated as raw bytes; print what we can as text and
            // fall back to a hex dump only if something non-UTF8 sneaks in.
            match core::str::from_utf8(&buf[..n]) {
                Ok(text) => {
                    // Avoid an extra blank line when the file already ends
                    // with a newline (hello.txt does).
                    if text.ends_with('\n') {
                        vga_print!("{}", text);
                        serial_print!("{}", text);
                    } else {
                        vga_println!("{}", text);
                        serial_println!("{}", text);
                    }
                }
                Err(_) => {
                    vga_println!("cat: {}: not valid UTF-8 ({} bytes)", path, n);
                    serial_println!("cat: {}: not valid UTF-8 ({} bytes)", path, n);
                }
            }
        }
        Err(FsError::NotFound) => {
            vga_println!("cat: {}: not found", path);
            serial_println!("cat: {}: not found", path);
        }
        Err(FsError::NotMounted) => {
            vga_println!("cat: filesystem not mounted");
            serial_println!("cat: filesystem not mounted");
        }
        Err(e) => {
            vga_println!("cat: {}: {}", path, e);
            serial_println!("cat: {}: {}", path, e);
        }
    }
}
