//! GOATos kernel entry point.
//!
//! This is intentionally minimal: it initializes serial output, prints proof
//! that the kernel booted, and then halts. It is the first "ground floor"
//! step for the OS — later work will add interrupts, memory management,
//! drivers, and everything else that turns this into a real operating system.
#![no_std]
#![no_main]

pub mod serial;

use bootloader_api::{entry_point, BootInfo};
use core::panic::PanicInfo;

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    serial::init();

    serial_println!("GOATos booted successfully!");
    serial_println!(
        "bootloader reported {} memory region(s)",
        boot_info.memory_regions.len()
    );

    hlt_loop();
}

fn hlt_loop() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("KERNEL PANIC: {}", info);
    hlt_loop();
}
