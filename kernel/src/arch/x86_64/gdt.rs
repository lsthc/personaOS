//! Global Descriptor Table + TSS for the BSP.
//!
//! Layout (index → usage):
//!     0: null
//!     1: kernel code (64-bit, ring 0)
//!     2: kernel data (ring 0)
//!     3: user data   (ring 3) — must come before user code for SYSRET.
//!     4: user code   (64-bit, ring 3)
//!     5,6: TSS (two slots; system descriptor is 16 bytes wide)
//!
//! Selector ordering matters: the `syscall/sysret` MSR `STAR` encodes the
//! user selectors as `{user_cs, user_ss} = {STAR[63:48]+16, STAR[63:48]+8}`,
//! so `user_ss` has to sit directly before `user_cs` in the GDT.

use core::mem::MaybeUninit;

use spin::Once;
use x86_64::instructions::segmentation::{Segment, CS, DS, ES, FS, GS, SS};
use x86_64::instructions::tables::load_tss;
use x86_64::registers::segmentation::SegmentSelector;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::PrivilegeLevel;
use x86_64::VirtAddr;

/// IST index for the double-fault handler. Value is baked into the IDT.
pub const DOUBLE_FAULT_IST: u16 = 0;

const STACK_SIZE: usize = 16 * 1024;

#[repr(align(16))]
struct Stack([u8; STACK_SIZE]);

static mut DF_STACK: Stack = Stack([0; STACK_SIZE]);
static mut PRIV0_STACK: Stack = Stack([0; STACK_SIZE]);

static GDT: Once<(GlobalDescriptorTable, Selectors)> = Once::new();
static TSS: Once<TaskStateSegment> = Once::new();

#[derive(Clone, Copy)]
pub struct Selectors {
    pub kernel_code: SegmentSelector,
    pub kernel_data: SegmentSelector,
    pub user_data: SegmentSelector,
    pub user_code: SegmentSelector,
    pub tss: SegmentSelector,
}

static mut SELECTORS: MaybeUninit<Selectors> = MaybeUninit::uninit();

/// Install the GDT + TSS on the current CPU.
///
/// # Safety
/// Must be called once per CPU, before the IDT is loaded.
pub unsafe fn init() {
    let tss = TSS.call_once(|| {
        let mut tss = TaskStateSegment::new();
        // IST 1 (index 0): a private stack for #DF, so we can still run if
        // the main kernel stack is wedged.
        let df_top = unsafe {
            let base = &raw const DF_STACK.0 as u64;
            VirtAddr::new(base + STACK_SIZE as u64)
        };
        tss.interrupt_stack_table[DOUBLE_FAULT_IST as usize] = df_top;
        // RSP0: the stack the CPU switches to on a ring-3 → ring-0 transition.
        let rsp0 = unsafe {
            let base = &raw const PRIV0_STACK.0 as u64;
            VirtAddr::new(base + STACK_SIZE as u64)
        };
        tss.privilege_stack_table[0] = rsp0;
        tss
    });

    let (gdt, sel) = GDT.call_once(|| {
        let mut g = GlobalDescriptorTable::new();
        let kernel_code = g.append(Descriptor::kernel_code_segment());
        let kernel_data = g.append(Descriptor::kernel_data_segment());
        let user_data = g.append(Descriptor::user_data_segment());
        let user_code = g.append(Descriptor::user_code_segment());
        let tss_sel = g.append(Descriptor::tss_segment(tss));
        (
            g,
            Selectors {
                kernel_code,
                kernel_data,
                user_data,
                user_code,
                tss: tss_sel,
            },
        )
    });

    GlobalDescriptorTable::load(gdt);
    unsafe {
        CS::set_reg(sel.kernel_code);
        DS::set_reg(sel.kernel_data);
        ES::set_reg(sel.kernel_data);
        SS::set_reg(sel.kernel_data);
        // FS/GS base stays untouched for now; GS is used later by per-CPU.
        FS::set_reg(SegmentSelector::new(0, PrivilegeLevel::Ring0));
        GS::set_reg(SegmentSelector::new(0, PrivilegeLevel::Ring0));
        load_tss(sel.tss);
        core::ptr::write(&raw mut SELECTORS, MaybeUninit::new(*sel));
    }
}

/// Fetch the selectors installed by [`init`]. Undefined before [`init`].
pub fn selectors() -> Selectors {
    unsafe { core::ptr::read(&raw const SELECTORS).assume_init() }
}

/// Update the ring-0 stack the CPU should switch to on the next
/// interrupt/syscall from user mode. Called by the scheduler when it picks a
/// user task to run.
pub fn set_kernel_stack(top: VirtAddr) {
    // SAFETY: TSS is initialized at this point and we only mutate the RSP0
    // field, which the CPU reads on ring-3 → ring-0 transitions. We're
    // running in ring 0 with interrupts off during context-switch.
    unsafe {
        let tss = TSS.get().unwrap() as *const _ as *mut TaskStateSegment;
        (*tss).privilege_stack_table[0] = top;
    }
}
