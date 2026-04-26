//! Legacy 8259 PIC — we mask every line and rely on the LAPIC exclusively.
//! Remap first to vectors 0x20..0x2F so spurious 8259 IRQs don't masquerade as
//! CPU exceptions if one ever slips through.

use crate::arch::x86_64::outb;

const PIC1_CMD: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_CMD: u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;

const ICW1_INIT: u8 = 0x10;
const ICW1_ICW4: u8 = 0x01;
const ICW4_8086: u8 = 0x01;

/// # Safety
/// Must be called with interrupts disabled, once, on the BSP.
pub unsafe fn disable_legacy() {
    unsafe {
        // Start init, then program vector offsets and cascade wiring.
        outb(PIC1_CMD, ICW1_INIT | ICW1_ICW4);
        outb(PIC2_CMD, ICW1_INIT | ICW1_ICW4);
        outb(PIC1_DATA, 0x20); // master offset
        outb(PIC2_DATA, 0x28); // slave offset
        outb(PIC1_DATA, 1 << 2); // slave on IRQ2
        outb(PIC2_DATA, 2);
        outb(PIC1_DATA, ICW4_8086);
        outb(PIC2_DATA, ICW4_8086);
        // Mask everything.
        outb(PIC1_DATA, 0xFF);
        outb(PIC2_DATA, 0xFF);
    }
}
