/* GOATos is loaded as a flat binary by our own boot.asm (no ELF loader, no
 * Multiboot) at a fixed physical/linear address, so this script just lays
 * sections out contiguously starting there. */
ENTRY(_start32)

SECTIONS {
    . = 0x10000;

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

    /DISCARD/ :
    {
        *(.eh_frame)
        *(.comment)
        *(.note*)
    }
}
