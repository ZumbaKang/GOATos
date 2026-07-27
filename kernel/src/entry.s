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

/* 64 KiB kernel stack in .bss. Grows down from stack_top. Exported so
 * memory::layout can name the range next to the heap and refuse to boot
 * quietly if the two ever overlap (roadmap 2.5). Keep the size in step with
 * KERNEL_STACK_SIZE in kernel/src/memory/layout.rs. */
.section .bss
.align 16
.global stack_bottom
.global stack_top
stack_bottom:
    .skip 65536 /* 64 KiB = KERNEL_STACK_SIZE */
stack_top:
