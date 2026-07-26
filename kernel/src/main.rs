//! GOATos kernel entry point.
//!
//! GOATos boots via its own hand-written, from-scratch bootloader (see
//! `boot/boot.asm`) - no GRUB, no Multiboot, no third-party boot code at
//! all. The bootloader loads this kernel as a flat binary, switches the CPU
//! to 32-bit protected mode, and jumps straight to `_start32` (`entry.s`),
//! which sets up a stack and calls into `kernel_main` below.
//!
//! This first step is intentionally minimal: it prints proof of boot to
//! both the VGA text screen (so it's *displayable*, on real hardware and in
//! a browser via v86) and the serial port (so headless QEMU/CI can verify
//! it booted), then halts.
#![no_std]
#![no_main]
// Interrupt/exception handlers are raw CPU entry points: they need the
// compiler to emit an `iret` epilogue and preserve every register, which is
// what the (still-unstable) `x86-interrupt` calling convention does. See
// `idt::Handler`.
#![feature(abi_x86_interrupt)]

use core::arch::{asm, global_asm};
use core::panic::PanicInfo;

pub mod gdt;
pub mod idt;
pub mod serial;
pub mod vga;

global_asm!(include_str!("entry.s"), options(att_syntax));

#[no_mangle]
pub extern "C" fn kernel_main() -> ! {
    // VGA is the primary, load-bearing output surface (it's what makes
    // GOATos "displayable" both on real hardware and via v86 in a browser),
    // so it comes first and never depends on the serial port succeeding.
    vga::clear_screen();
    vga_println!("GOATos");
    vga_println!("------");
    vga_println!("Booted successfully via a from-scratch bootloader.");
    vga_println!("This screen is the same VGA text buffer real BIOS");
    vga_println!("hardware (and browser emulators like v86) render -");
    vga_println!("what you see here is what a web visitor would see.");

    // Take over segmentation from the bootloader's throwaway GDT. Printing
    // first means a botched GDT load shows up as "the banner is on screen but
    // nothing after it" rather than as a completely blank screen.
    gdt::init();
    let gdt = gdt::loaded();
    vga_println!("");
    vga_println!(
        "GDT: kernel-owned at {:#010x} (limit {:#x}), cs={:#04x} ds={:#04x}",
        gdt.base,
        gdt.limit,
        gdt.cs,
        gdt.ds
    );

    // Load an (empty) IDT of the kernel's own. Interrupts stay masked and no
    // vector has a handler yet, so nothing should route through it - the point
    // of doing it now is that everything after this line can be given a real
    // fault handler instead of freezing.
    idt::init();
    let idt = idt::loaded();
    vga_println!(
        "IDT: {} entries at {:#010x} (limit {:#x}), {} handlers",
        idt.entries(),
        idt.base,
        idt.limit,
        idt.handlers
    );

    // Serial output is best-effort: useful for headless QEMU/CI, but its
    // absence must never affect anything above.
    serial::init();
    serial_println!("GOATos booted successfully! (32-bit, from a hand-written bootloader)");
    serial_println!(
        "GDT: kernel-owned at {:#010x} (limit {:#x}), cs={:#04x} ds={:#04x}",
        gdt.base,
        gdt.limit,
        gdt.cs,
        gdt.ds
    );
    serial_println!(
        "IDT: {} entries at {:#010x} (limit {:#x}), {} handlers",
        idt.entries(),
        idt.base,
        idt.limit,
        idt.handlers
    );

    hlt_loop();
}

fn hlt_loop() -> ! {
    loop {
        unsafe { asm!("hlt") };
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("KERNEL PANIC: {}", info);
    hlt_loop();
}
