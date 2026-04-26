//! x86_64 architecture support — low-level primitives and subsystem bring-up.

use core::arch::asm;

pub mod acpi;
pub mod apic;
pub mod gdt;
pub mod idt;
pub mod pic;
pub mod syscall;

/// Halt the CPU until the next interrupt. Used by idle loops.
#[inline(always)]
pub fn halt() {
    unsafe {
        asm!("hlt", options(nomem, nostack, preserves_flags));
    }
}

#[inline(always)]
pub fn cli() {
    unsafe {
        asm!("cli", options(nomem, nostack, preserves_flags));
    }
}

#[inline(always)]
pub fn sti() {
    unsafe {
        asm!("sti", options(nomem, nostack, preserves_flags));
    }
}

#[inline(always)]
pub unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    unsafe {
        asm!("in al, dx", out("al") val, in("dx") port, options(nomem, nostack, preserves_flags));
    }
    val
}

#[inline(always)]
pub unsafe fn outb(port: u16, val: u8) {
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack, preserves_flags));
    }
}

#[inline(always)]
#[allow(dead_code)]
pub unsafe fn inw(port: u16) -> u16 {
    let val: u16;
    unsafe {
        asm!("in ax, dx", out("ax") val, in("dx") port, options(nomem, nostack, preserves_flags));
    }
    val
}

#[inline(always)]
#[allow(dead_code)]
pub unsafe fn outw(port: u16, val: u16) {
    unsafe {
        asm!("out dx, ax", in("dx") port, in("ax") val, options(nomem, nostack, preserves_flags));
    }
}

#[inline(always)]
pub unsafe fn rdmsr(msr: u32) -> u64 {
    let (low, high): (u32, u32);
    unsafe {
        asm!(
            "rdmsr",
            in("ecx") msr,
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags),
        );
    }
    ((high as u64) << 32) | low as u64
}

#[inline(always)]
pub unsafe fn wrmsr(msr: u32, value: u64) {
    let low = value as u32;
    let high = (value >> 32) as u32;
    unsafe {
        asm!(
            "wrmsr",
            in("ecx") msr,
            in("eax") low,
            in("edx") high,
            options(nomem, nostack, preserves_flags),
        );
    }
}

/// Bring up the architecture-specific kernel state: GDT+TSS, IDT, legacy PIC
/// disabled, LAPIC online with a periodic timer, syscall MSRs programmed.
/// Must run once on the BSP before enabling interrupts.
///
/// # Safety
///
/// Caller must ensure interrupts are currently disabled and the heap is
/// already online (GDT allocates its kernel interrupt stack).
pub unsafe fn init_bsp() {
    unsafe {
        gdt::init();
        idt::init();
        pic::disable_legacy();
        apic::init_bsp();
        syscall::init_bsp();
    }
}
