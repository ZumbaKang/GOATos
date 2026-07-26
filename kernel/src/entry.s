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

.section .bss
.align 16
stack_bottom:
    .skip 65536 /* 64 KiB kernel stack */
stack_top:
