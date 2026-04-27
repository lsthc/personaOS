//! Local APIC (xAPIC) — enable, calibrate timer against the PIT, set periodic tick.
//!
//! xAPIC registers are MMIO-mapped at the physical address in `IA32_APIC_BASE`;
//! we access them through the HHDM. This is a BSP-only bring-up for M1.

use core::sync::atomic::{AtomicPtr, Ordering};

use persona_shared::HHDM_OFFSET;

use crate::arch::x86_64::{inb, outb, rdmsr, wrmsr};

const IA32_APIC_BASE: u32 = 0x1B;
const APIC_BASE_ENABLE: u64 = 1 << 11;

// Register offsets (bytes from LAPIC base).
const REG_ID: usize = 0x020;
const REG_EOI: usize = 0x0B0;
const REG_SIVR: usize = 0x0F0;
const REG_LVT_TIMER: usize = 0x320;
const REG_TIMER_ICR: usize = 0x380;
const REG_TIMER_CCR: usize = 0x390;
const REG_TIMER_DCR: usize = 0x3E0;

const LVT_MASKED: u32 = 1 << 16;
const LVT_TIMER_PERIODIC: u32 = 1 << 17;

const DCR_DIV_16: u32 = 0b0011;

static LAPIC_BASE: AtomicPtr<u32> = AtomicPtr::new(core::ptr::null_mut());

#[inline]
fn reg(off: usize) -> *mut u32 {
    let base = LAPIC_BASE.load(Ordering::Relaxed);
    // SAFETY: base is set once during init to a 16-byte-aligned MMIO region;
    // `off` is a fixed offset within the 4 KiB LAPIC window.
    unsafe { (base as *mut u8).add(off) as *mut u32 }
}

#[inline]
fn read(off: usize) -> u32 {
    unsafe { reg(off).read_volatile() }
}

#[inline]
fn write(off: usize, val: u32) {
    unsafe { reg(off).write_volatile(val) }
}

/// End-of-interrupt — must be written by every IRQ handler except the
/// spurious vector.
pub fn eoi() {
    write(REG_EOI, 0);
}

/// Bring up the LAPIC on the BSP with a periodic tick at ~100 Hz.
///
/// # Safety
/// Must be called once, with interrupts disabled, after IDT is loaded.
pub unsafe fn init_bsp() {
    let apic_base_msr = unsafe { rdmsr(IA32_APIC_BASE) };
    let phys = apic_base_msr & 0xFFFF_F000;
    let virt = (phys + HHDM_OFFSET) as *mut u32;
    LAPIC_BASE.store(virt, Ordering::Relaxed);

    unsafe {
        wrmsr(IA32_APIC_BASE, apic_base_msr | APIC_BASE_ENABLE);
    }

    // Spurious vector + software enable.
    write(
        REG_SIVR,
        0x100 | crate::arch::x86_64::idt::SPURIOUS_VECTOR as u32,
    );

    // Calibrate: count LAPIC timer ticks over a known PIT interval (~10 ms).
    write(REG_TIMER_DCR, DCR_DIV_16);
    unsafe { pit_oneshot_ms(10) };
    write(REG_TIMER_ICR, u32::MAX);
    unsafe { pit_wait() };
    let remaining = read(REG_TIMER_CCR);
    let elapsed = u32::MAX - remaining;
    // Ticks per 10 ms → multiply by 1 for 10 ms period; we want 100 Hz.
    let period = elapsed.max(1);

    write(
        REG_LVT_TIMER,
        LVT_TIMER_PERIODIC | crate::arch::x86_64::idt::TIMER_VECTOR as u32,
    );
    write(REG_TIMER_ICR, period);
}

/// Mask the LAPIC timer. Used by tests that can't tolerate preemption.
#[allow(dead_code)]
pub fn mask_timer() {
    write(REG_LVT_TIMER, LVT_MASKED);
}

#[allow(dead_code)]
pub fn id() -> u32 {
    read(REG_ID) >> 24
}

// ---------------------------------------------------------------------------
// PIT-based calibration
// ---------------------------------------------------------------------------

const PIT_CH2: u16 = 0x42;
const PIT_CMD: u16 = 0x43;
const PIT_GATE: u16 = 0x61;
const PIT_HZ: u32 = 1_193_182;

unsafe fn pit_oneshot_ms(ms: u32) {
    let ticks = (PIT_HZ as u64 * ms as u64 / 1000) as u16;
    unsafe {
        // Gate channel 2: disable speaker output, enable gate.
        let gate = inb(PIT_GATE);
        outb(PIT_GATE, (gate & !0x02) | 0x01);
        // Channel 2, lobyte/hibyte, mode 0 (one-shot), binary.
        outb(PIT_CMD, 0b10110000);
        outb(PIT_CH2, ticks as u8);
        outb(PIT_CH2, (ticks >> 8) as u8);
    }
}

unsafe fn pit_wait() {
    unsafe {
        while inb(PIT_GATE) & 0x20 == 0 {
            core::hint::spin_loop();
        }
    }
}
