/* Linker script for personaOS userspace binaries (init).
 *
 * Linked at 0x40_0000 — the USER_TEXT_VIRT constant the kernel's user
 * loader expects for the first PT_LOAD segment. PIE would also work, but
 * static linking is simpler for the one binary we run today.
 */

ENTRY(_start)

USER_BASE = 0x400000;

SECTIONS
{
    . = USER_BASE;

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