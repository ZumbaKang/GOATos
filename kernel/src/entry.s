/* Kernel entry stub.
 *
 * boot.asm has already switched the CPU into 32-bit protected mode and
 * jumped straight to the first byte of this binary (loaded at a fixed
 * physical address - see KERNEL_LOAD_ADDR in boot.asm and `. = 0x10000` in
 * linker.ld). All that's left is to set up a stack before calling into
 * Rust, since the CPU doesn't have a valid one yet.
 */

/* Its own named section, explicitly placed first in linker.ld - the raw
 * flat-binary loader in boot.asm jumps straight to the start of the loaded
 * image, so whatever code is first in the final binary IS the entry point,
 * regardless of where the linker would otherwise have placed it. */
.section .entry, "ax", @progbits
.global _start32
.type _start32, @function
_start32:
    mov $stack_top, %esp
    mov %esp, %ebp

    call kernel_main

.Lhang:
    hlt
    jmp .Lhang

/* Stacks and the guard page that separates them, all page-aligned so the
 * identity map can leave exactly one 4 KiB PTE not-present.
 *
 * Layout (low -> high):
 *   [double_fault_stack_bottom, double_fault_stack_top)  4 KiB  DF handler
 *   [stack_guard_page,          stack_bottom)            4 KiB  unmapped
 *   [stack_bottom,              stack_top)              64 KiB  kernel
 *
 * The kernel stack grows down from stack_top; overflowing it hits the
 * unmapped guard and faults instead of scribbling into the DF stack (or
 * anything else that happens to sit below). Keep the sizes in step with
 * DOUBLE_FAULT_STACK_SIZE / GUARD_PAGE_SIZE / KERNEL_STACK_SIZE in
 * kernel/src/memory/layout.rs. */
.section .bss
.align 4096
.global double_fault_stack_bottom
.global double_fault_stack_top
.global stack_guard_page
.global stack_bottom
.global stack_top
double_fault_stack_bottom:
    .skip 4096 /* 4 KiB = DOUBLE_FAULT_STACK_SIZE */
double_fault_stack_top:
stack_guard_page:
    .skip 4096 /* 4 KiB = GUARD_PAGE_SIZE; left unmapped by paging::init */
stack_bottom:
    .skip 65536 /* 64 KiB = KERNEL_STACK_SIZE */
stack_top:
