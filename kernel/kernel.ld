/* Linker script for the personaOS kernel.
 *
 * We link the kernel at -2 GiB (0xFFFFFFFF80000000) which matches the
 * "kernel" code model used by the compiler. This is the canonical higher
 * half negative-address range.
 */

ENTRY(_start)

KERNEL_BASE = 0xFFFFFFFF80000000;

SECTIONS
{
    . = KERNEL_BASE;

    .text : ALIGN(4K) {
        KEEP(*(.text._start))
        *(.text .text.*)
    }

    .rodata : ALIGN(4K) {
        *(.rodata .rodata.*)
    }

    .data : ALIGN(4K) {
        *(.data .data.*)
    }

    .bss : ALIGN(4K) {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        __bss_end = .;
    }

    /DISCARD/ : {
        *(.eh_frame)
        *(.note .note.*)
        *(.comment)
    }
}
