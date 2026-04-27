//! Interrupt Descriptor Table for the BSP.
//!
//! Vectors 0..31 are CPU exceptions; 32+ are external / software.
//!   - 0x20 (32): LAPIC timer tick (scheduler preemption).
//!   - 0xFF (255): LAPIC spurious interrupt — required by SDM.

use core::cell::UnsafeCell;
use core::fmt::Write as _;

use spin::{Mutex, Once};
use x86_64::registers::control::Cr2;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};

use crate::arch::x86_64::{gdt, halt};
use crate::drivers::serial::SerialPort;

pub const TIMER_VECTOR: u8 = 0x20;
pub const SPURIOUS_VECTOR: u8 = 0xFF;

/// Vector allocation pool for dynamically installed IRQ handlers (MSI/MSI-X).
/// Bits 0x30..=0xEF are available; 0x20 (timer) and 0xFF (spurious) stay
/// reserved out-of-band. 32 bytes × 8 bits covers 0..=0xFF; we only touch
/// the pool range.
const DYN_VECTOR_LO: u8 = 0x30;
const DYN_VECTOR_HI: u8 = 0xEF;
static VECTOR_BITMAP: Mutex<[u8; 32]> = Mutex::new([0; 32]);

/// The IDT itself, wrapped so we can install handlers at runtime after the
/// table has been loaded. The CPU reads descriptors directly out of this
/// backing store on every interrupt, so as long as the address stays stable
/// (it does — the cell lives forever inside `Once`), late mutations are
/// observed correctly.
struct IdtCell(UnsafeCell<InterruptDescriptorTable>);
// SAFETY: writers hold `VECTOR_BITMAP` for serialization; the CPU reads
// descriptors atomically per-vector.
unsafe impl Sync for IdtCell {}

static IDT: Once<IdtCell> = Once::new();

/// Load the IDT on the current CPU.
///
/// # Safety
/// GDT must already be installed (the double-fault IST index we hand to the
/// CPU is only valid once the TSS is live).
pub unsafe fn init() {
    let cell = IDT.call_once(|| {
        let mut idt = InterruptDescriptorTable::new();

        idt.divide_error.set_handler_fn(ex_divide);
        idt.debug.set_handler_fn(ex_debug);
        idt.non_maskable_interrupt.set_handler_fn(ex_nmi);
        idt.breakpoint.set_handler_fn(ex_breakpoint);
        idt.overflow.set_handler_fn(ex_overflow);
        idt.bound_range_exceeded.set_handler_fn(ex_bound);
        idt.invalid_opcode.set_handler_fn(ex_invalid_opcode);
        idt.device_not_available.set_handler_fn(ex_device_na);
        // The double-fault handler runs on its own stack via IST.
        unsafe {
            idt.double_fault
                .set_handler_fn(ex_double_fault)
                .set_stack_index(gdt::DOUBLE_FAULT_IST);
        }
        idt.invalid_tss.set_handler_fn(ex_invalid_tss);
        idt.segment_not_present.set_handler_fn(ex_seg_not_present);
        idt.stack_segment_fault.set_handler_fn(ex_stack_fault);
        idt.general_protection_fault.set_handler_fn(ex_gpf);
        idt.page_fault.set_handler_fn(ex_page_fault);
        idt.x87_floating_point.set_handler_fn(ex_x87);
        idt.alignment_check.set_handler_fn(ex_alignment);
        idt.machine_check.set_handler_fn(ex_machine_check);
        idt.simd_floating_point.set_handler_fn(ex_simd);
        idt.virtualization.set_handler_fn(ex_virt);

        idt[TIMER_VECTOR].set_handler_fn(irq_timer);
        idt[SPURIOUS_VECTOR].set_handler_fn(irq_spurious);

        IdtCell(UnsafeCell::new(idt))
    });
    // SAFETY: `call_once` has produced a pinned IDT; loading a reference
    // derived from the UnsafeCell is fine because `load()` only copies the
    // base/limit into IDTR and returns.
    unsafe { (*cell.0.get()).load() };
}

/// Reserve an unused IRQ vector and install `handler` on it. Returned value
/// is a valid 8-bit vector in 0x30..=0xEF. Panics if the pool is exhausted.
///
/// The handler runs in `x86-interrupt` calling convention; like `irq_timer`
/// it must call `apic::eoi()` before returning.
#[allow(dead_code)] // consumed by NVMe / xHCI MSI-X setup
pub fn alloc_vector(handler: extern "x86-interrupt" fn(InterruptStackFrame)) -> u8 {
    let mut bm = VECTOR_BITMAP.lock();
    for v in DYN_VECTOR_LO..=DYN_VECTOR_HI {
        let byte = (v >> 3) as usize;
        let bit = v & 7;
        if bm[byte] & (1 << bit) == 0 {
            bm[byte] |= 1 << bit;
            // SAFETY: IDT backing store is live (init ran before we can
            // possibly be called), and we hold the bitmap lock so no one else
            // is touching this vector concurrently.
            unsafe {
                let cell = IDT.get().expect("IDT not initialized");
                let idt = &mut *cell.0.get();
                idt[v].set_handler_fn(handler);
            }
            return v;
        }
    }
    panic!("alloc_vector: no free IRQ vectors");
}

fn serial() -> SerialPort {
    // SAFETY: 0x3F8 is the standard COM1 port; the UART was initialized at
    // boot. We're re-wrapping it for a transient write; no shared state.
    unsafe { SerialPort::new(0x3F8) }
}

fn report_and_halt(name: &str, frame: &InterruptStackFrame) -> ! {
    let mut s = serial();
    let _ = writeln!(
        s,
        "\n[kernel] EXCEPTION: {} at rip={:#x} cs={:#x} rsp={:#x} rflags={:#x}",
        name,
        frame.instruction_pointer.as_u64(),
        frame.code_segment.0,
        frame.stack_pointer.as_u64(),
        frame.cpu_flags.bits(),
    );
    loop {
        halt();
    }
}

fn report_with_err(name: &str, frame: &InterruptStackFrame, err: u64) -> ! {
    let mut s = serial();
    let _ = writeln!(
        s,
        "\n[kernel] EXCEPTION: {} err={:#x} rip={:#x} cs={:#x}",
        name,
        err,
        frame.instruction_pointer.as_u64(),
        frame.code_segment.0,
    );
    loop {
        halt();
    }
}

// ---------------------------------------------------------------------------
// CPU exception handlers
// ---------------------------------------------------------------------------

extern "x86-interrupt" fn ex_divide(frame: InterruptStackFrame) {
    report_and_halt("#DE divide error", &frame);
}

extern "x86-interrupt" fn ex_debug(frame: InterruptStackFrame) {
    report_and_halt("#DB debug", &frame);
}

extern "x86-interrupt" fn ex_nmi(frame: InterruptStackFrame) {
    report_and_halt("NMI", &frame);
}

extern "x86-interrupt" fn ex_breakpoint(frame: InterruptStackFrame) {
    let mut s = serial();
    let _ = writeln!(
        s,
        "[kernel] #BP breakpoint at {:#x}",
        frame.instruction_pointer.as_u64()
    );
}

extern "x86-interrupt" fn ex_overflow(frame: InterruptStackFrame) {
    report_and_halt("#OF overflow", &frame);
}

extern "x86-interrupt" fn ex_bound(frame: InterruptStackFrame) {
    report_and_halt("#BR bound range", &frame);
}

extern "x86-interrupt" fn ex_invalid_opcode(frame: InterruptStackFrame) {
    report_and_halt("#UD invalid opcode", &frame);
}

extern "x86-interrupt" fn ex_device_na(frame: InterruptStackFrame) {
    report_and_halt("#NM device not available", &frame);
}

extern "x86-interrupt" fn ex_double_fault(frame: InterruptStackFrame, err: u64) -> ! {
    report_with_err("#DF double fault", &frame, err);
}

extern "x86-interrupt" fn ex_invalid_tss(frame: InterruptStackFrame, err: u64) {
    report_with_err("#TS invalid TSS", &frame, err);
}

extern "x86-interrupt" fn ex_seg_not_present(frame: InterruptStackFrame, err: u64) {
    report_with_err("#NP segment not present", &frame, err);
}

extern "x86-interrupt" fn ex_stack_fault(frame: InterruptStackFrame, err: u64) {
    report_with_err("#SS stack fault", &frame, err);
}

extern "x86-interrupt" fn ex_gpf(frame: InterruptStackFrame, err: u64) {
    report_with_err("#GP general protection", &frame, err);
}

extern "x86-interrupt" fn ex_page_fault(frame: InterruptStackFrame, err: PageFaultErrorCode) {
    let cr2 = Cr2::read();
    let mut s = serial();
    let _ = writeln!(
        s,
        "\n[kernel] EXCEPTION: #PF page fault cr2={:?} err={:?} rip={:#x}",
        cr2,
        err,
        frame.instruction_pointer.as_u64(),
    );
    loop {
        halt();
    }
}

extern "x86-interrupt" fn ex_x87(frame: InterruptStackFrame) {
    report_and_halt("#MF x87 FP", &frame);
}

extern "x86-interrupt" fn ex_alignment(frame: InterruptStackFrame, err: u64) {
    report_with_err("#AC alignment", &frame, err);
}

extern "x86-interrupt" fn ex_machine_check(frame: InterruptStackFrame) -> ! {
    report_and_halt("#MC machine check", &frame);
}

extern "x86-interrupt" fn ex_simd(frame: InterruptStackFrame) {
    report_and_halt("#XM SIMD", &frame);
}

extern "x86-interrupt" fn ex_virt(frame: InterruptStackFrame) {
    report_and_halt("#VE virtualization", &frame);
}

// ---------------------------------------------------------------------------
// External interrupts
// ---------------------------------------------------------------------------

extern "x86-interrupt" fn irq_timer(_frame: InterruptStackFrame) {
    crate::arch::x86_64::apic::eoi();
    crate::sched::on_tick();
}

extern "x86-interrupt" fn irq_spurious(_frame: InterruptStackFrame) {
    // Spurious vector — per SDM no EOI.
}
