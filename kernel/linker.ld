/* GOATos is loaded as a flat binary by our own boot.asm (no ELF loader, no
 * Multiboot) at a fixed physical/linear address, so this script just lays
 * sections out contiguously starting there. */
ENTRY(_start32)

SECTIONS {
    . = 0x10000;

    /* The physical extent of the loaded kernel, for the frame allocator to
     * exclude from the memory it hands out. __kernel_end is past .bss, which
     * is *not* part of the flat binary (objcopy emits nothing for NOBITS) but
     * is very much part of what the kernel occupies in RAM - the DF stack,
     * stack guard page, and 64KiB kernel stack from entry.s live there. */
    __kernel_start = .;

    .text :
    {
        /* _start32 (entry.s) must be the very first thing in the image:
         * boot.asm jumps straight to the load address with no ELF/symbol
         * awareness. */
        KEEP(*(.entry))
        *(.text .text.*)
    }

    .rodata :
    {
        *(.rodata .rodata.*)
    }

    .data :
    {
        *(.data .data.*)
    }

    .bss :
    {
        *(.bss .bss.*)
        *(COMMON)
    }

    __kernel_end = .;

    /DISCARD/ :
    {
        *(.eh_frame)
        *(.comment)
        *(.note*)
    }
}
