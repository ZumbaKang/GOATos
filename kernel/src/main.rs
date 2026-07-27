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
// `alloc` needs a crate-level handler for OOM; without it a failed `Vec`
// push would hit an unresolved symbol instead of our panic path.
#![feature(alloc_error_handler)]

extern crate alloc;

use core::alloc::Layout;
use core::arch::{asm, global_asm};
use core::panic::PanicInfo;

pub mod exceptions;
pub mod gdt;
pub mod idt;
pub mod interrupts;
pub mod keyboard;
pub mod memory;
pub mod pic;
pub mod pit;
pub mod serial;
pub mod sync;
pub mod tss;
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

    // Serial output is best-effort: useful for headless QEMU/CI, but its
    // absence must never affect anything above. It comes up before the
    // subsystems below so that a fault in any of them still has a headless
    // surface to report itself on (see `exceptions`).
    serial::init();

    // Take over segmentation from the bootloader's throwaway GDT. Printing
    // first means a botched GDT load shows up as "the banner is on screen but
    // nothing after it" rather than as a completely blank screen.
    //
    // The TSSes come first because the GDT holds descriptors for them, and
    // loading the task register (which `gdt::init` does) reads one.
    tss::init(exceptions::double_fault_entry);
    gdt::init();
    let gdt = gdt::loaded();
    vga_println!("");
    vga_println!(
        "GDT: kernel-owned at {:#010x} (limit {:#x}), cs={:#04x} ds={:#04x} tr={:#04x}",
        gdt.base,
        gdt.limit,
        gdt.cs,
        gdt.ds,
        gdt.tr
    );

    // Load an IDT of the kernel's own, then fill in the exception vectors an
    // ordinary kernel bug is most likely to hit. Interrupts are still masked,
    // so nothing but a fault can route through it yet - which is the point:
    // from here on, a bug in the code below reports itself instead of
    // triple-faulting the machine into a silent reboot.
    idt::init();
    exceptions::init();
    let idt = idt::loaded();
    vga_println!(
        "IDT: {} entries at {:#010x} (limit {:#x}), {} handlers",
        idt.entries(),
        idt.base,
        idt.limit,
        idt.handlers
    );
    vga_println!("{}", exceptions::INSTALLED_SUMMARY);
    let (df_stack_bottom, df_stack_top) = tss::double_fault_stack_range();
    vga_println!(
        "Double fault: task gate -> TSS {:#04x}, stack {:#010x}..{:#010x}",
        gdt::DOUBLE_FAULT_TSS_SELECTOR,
        df_stack_bottom,
        df_stack_top
    );

    // Move the hardware IRQs off the exception vectors they collide with by
    // default, and leave every line masked: there is still no handler for any
    // of them, and interrupts are still disabled. This has to happen before
    // interrupts are ever enabled, which is why it lands with the exception
    // work rather than with the first driver that wants an IRQ.
    pic::init();
    let pic = pic::state();
    let pic_masking = if pic.all_masked() {
        "all masked"
    } else {
        "SOME IRQS UNMASKED"
    };
    vga_println!(
        "PIC: IRQ0-15 -> vectors {}-{}, IMR {:#04x}/{:#04x} ({})",
        pic.vector_base,
        pic.vector_last,
        pic.master_mask,
        pic.slave_mask,
        pic_masking
    );

    // Give every remaining vector somewhere to land before letting the CPU
    // deliver anything: an interrupt on a vector with no gate would escalate
    // straight to a double fault, so "interrupts on" and "no gaps in the IDT"
    // have to happen together.
    let spare_vectors = interrupts::init();
    vga_println!(
        "Interrupts: catch-all on {} spare vectors, {} owned",
        spare_vectors,
        idt::ENTRY_COUNT - spare_vectors
    );

    // What the bootloader asked the BIOS before protected mode put it out of
    // reach: the machine's physical memory layout, which is where every
    // allocation this kernel will ever make ultimately comes from.
    let memory = memory::map::load();
    vga_println!("{}", memory);

    // Divide the usable part of that map into 4 KiB frames, minus the ranges
    // this kernel has already put something in, and prove on every boot that
    // the result hands out distinct, real frames and takes them back.
    let frames = memory::frame::init(&memory);
    vga_println!("{}", frames);
    let frame_test = memory::frame::self_test();
    vga_print!("Frames:");
    for frame in frame_test.frames() {
        vga_print!(" {:08x}", frame.start_address());
    }
    vga_println!("");
    vga_println!("{}", frame_test);

    serial_println!(
        "GDT: kernel-owned at {:#010x} (limit {:#x}), cs={:#04x} ds={:#04x} tr={:#04x}",
        gdt.base,
        gdt.limit,
        gdt.cs,
        gdt.ds,
        gdt.tr
    );
    serial_println!(
        "IDT: {} entries at {:#010x} (limit {:#x}), {} handlers",
        idt.entries(),
        idt.base,
        idt.limit,
        idt.handlers
    );
    serial_println!("{}", exceptions::INSTALLED_SUMMARY);
    serial_println!(
        "Double fault: task gate -> TSS {:#04x}, stack {:#010x}..{:#010x}",
        gdt::DOUBLE_FAULT_TSS_SELECTOR,
        df_stack_bottom,
        df_stack_top
    );
    serial_println!(
        "PIC: IRQ0-15 -> vectors {}-{}, IMR {:#04x}/{:#04x} ({})",
        pic.vector_base,
        pic.vector_last,
        pic.master_mask,
        pic.slave_mask,
        pic_masking
    );
    serial_println!(
        "Interrupts: catch-all on {} spare vectors, {} owned",
        spare_vectors,
        idt::ENTRY_COUNT - spare_vectors
    );
    // The screen only has room for the summary, but serial has room for the
    // whole map - and the individual regions are the part worth eyeballing
    // against the RAM size the machine was configured with.
    for region in memory.regions() {
        serial_println!(
            "E820:   {:#012x}-{:#012x} {:>9} KiB  {}",
            region.base,
            region.end(),
            region.length / 1024,
            region.kind
        );
    }
    serial_println!("{}", memory);

    for reservation in frames.reservations() {
        serial_println!("Frames: reserved {}", reservation);
    }
    serial_println!("{}", frames);
    // The addresses themselves, one per line, so the boot log carries the
    // evidence for the summary rather than just the summary: eight distinct
    // frames, none of them inside any of the reserved ranges above.
    for (position, frame) in frame_test.frames().iter().enumerate() {
        serial_println!("Frame:  #{} at {}", position + 1, frame);
    }
    for frame in frame_test.freed() {
        serial_println!("Frame:  freed {}", frame);
    }
    if let Some(frame) = frame_test.reused() {
        serial_println!("Frame:  reused {} for the next allocation", frame);
    }
    serial_println!("{}", frame_test);

    // Identity-map everything the kernel already touches and flip `CR0.PG`.
    // Until this point every address was physical; afterwards the same
    // numbers still work, but only because the page tables say so. Runs
    // before `sti` so a botched map faults with interrupts still masked.
    let paging = memory::paging::init(&memory);
    vga_println!("{}", paging);
    serial_println!("{}", paging);

    // Carve a contiguous run of frames into a heap and install it as the
    // global allocator. Needs paging on first only in the dependency sense
    // (identity map already covers the frames); the self-test is what proves
    // `alloc::vec::Vec` actually works end to end.
    let heap = memory::heap::init();
    vga_println!("{}", heap);
    serial_println!("{}", heap);
    let heap_test = memory::heap::self_test();
    vga_println!("{}", heap_test);
    serial_println!("{}", heap_test);

    // Pin the heap/stack/guard layout in writing and check the ranges are
    // still right. The stacks and the unmapped guard live inside the reserved
    // kernel image; the heap is carved from free frames outside it - so by
    // construction they cannot collide, and a failure here means that
    // construction broke.
    let layout = memory::layout::check(heap);
    vga_println!("{}", layout);
    serial_println!("{}", layout);
    // Also dump the absolute ranges over serial so a CI script can re-derive
    // adjacency/disjointness itself, rather than trusting the kernel's verdict.
    serial_println!("Layout: image {}", layout.kernel_image);
    serial_println!("Layout: dfstack {}", layout.double_fault_stack);
    serial_println!("Layout: guard {}", layout.guard_page);
    serial_println!("Layout: kstack {}", layout.kernel_stack);
    serial_println!("Layout: heap {}", layout.heap);

    // The last piece of setup, and the first moment the kernel can be
    // interrupted at all. The masks are re-read afterwards because they are
    // what makes this uneventful: every IRQ line is still disabled, so nothing
    // is delivered until a driver asks for its own line.
    interrupts::enable();
    let pic = pic::state();
    diag_println!(
        "Interrupts: enabled (IF={}), IMR {:#04x}/{:#04x}, {} unhandled",
        u8::from(interrupts::enabled()),
        pic.master_mask,
        pic.slave_mask,
        interrupts::unhandled_count()
    );

    serial_println!("GOATos booted successfully! (32-bit, from a hand-written bootloader)");

    // Compile to nothing unless a `trigger-*` feature is enabled, which is how
    // the handlers / print path above get verified against a real exception, a
    // real unexpected interrupt, or a real mid-print re-entry. Runs *before*
    // the PIT unmasks IRQ0, so a timer tick cannot interleave with a deliberate
    // fault/re-entry probe.
    exceptions::trigger_debug_exception();
    interrupts::trigger_debug_interrupt();
    interrupts::trigger_print_reentrancy();

    // First real IRQ line: program the PIT, replace the catch-all on its
    // vector, and unmask IRQ0. The masks reported just above were still
    // 0xff/0xff; this is what changes that.
    pit::init();
    let pic = pic::state();
    diag_println!(
        "PIT: channel 0 at {} Hz (IRQ0 -> vector {}, divisor {}), IMR {:#04x}/{:#04x}",
        pit::FREQUENCY_HZ,
        pit::VECTOR,
        pit::DIVISOR,
        pic.master_mask,
        pic.slave_mask
    );

    // Second real IRQ line: PS/2 keyboard on IRQ1. Typing echoes straight to
    // VGA (and serial) from the handler - proof the translate path works
    // before roadmap 3.3 introduces a proper input queue.
    keyboard::init();
    let pic = pic::state();
    diag_println!(
        "Keyboard: PS/2 on IRQ1 -> vector {}, IMR {:#04x}/{:#04x} (type to echo)",
        keyboard::VECTOR,
        pic.master_mask,
        pic.slave_mask
    );

    // Idle: sleep until the next IRQ, and once a second prove the tick
    // counter is actually advancing (the "Done when" for roadmap 3.1).
    // Keyboard echoes arrive from IRQ1 on their own and do not need the loop.
    idle_with_timer();
}

/// Parks the CPU for good, waking only to halt again. Used by an exception
/// handler that has finished reporting (and by any path that must stop cold).
pub fn hlt_loop() -> ! {
    loop {
        unsafe { asm!("hlt") };
    }
}

/// Idles like [`hlt_loop`], but wakes on each timer tick and reports the
/// counter once per second over serial - proof that IRQ0 is firing.
fn idle_with_timer() -> ! {
    let mut last_second = 0u32;
    loop {
        let second = pit::seconds();
        if second > last_second {
            last_second = second;
            // Serial is what CI greps; VGA gets the same line so a screendump
            // (or the web demo) shows the counter moving too.
            diag_println!("PIT: tick {} ({} s)", pit::ticks(), second);
        }
        unsafe { asm!("hlt") };
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("KERNEL PANIC: {}", info);
    hlt_loop();
}

#[alloc_error_handler]
fn alloc_error(layout: Layout) -> ! {
    panic!("allocation failed: size={} align={}", layout.size(), layout.align());
}
