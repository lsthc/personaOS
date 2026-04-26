//! x86_64 architecture support.

use core::arch::asm;

/// Halt the CPU until the next interrupt. Used by idle loops.
#[inline(always)]
pub fn halt() {
    unsafe {
        asm!("hlt", options(nomem, nostack, preserves_flags));
    }
}

/// Read an 8-bit value from a port.
#[inline(always)]
pub unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    unsafe {
        asm!("in al, dx", out("al") val, in("dx") port, options(nomem, nostack, preserves_flags));
    }
    val
}

/// Write an 8-bit value to a port.
#[inline(always)]
pub unsafe fn outb(port: u16, val: u8) {
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack, preserves_flags));
    }
}
