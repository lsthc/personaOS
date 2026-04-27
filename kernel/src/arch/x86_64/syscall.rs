//! `syscall` / `sysret` fast-path bring-up.
//!
//! Userland enters the kernel by executing `syscall`. The CPU loads CS/SS
//! from `STAR`, RIP from `LSTAR`, and masks the flags in `SFMASK`. Our entry
//! stub saves the user context onto the current task's kernel stack and hands
//! off to a Rust dispatcher; on return we restore and `sysretq` back to ring 3.

use core::arch::naked_asm;

use core::sync::atomic::{AtomicU64, Ordering};

use x86_64::VirtAddr;

use crate::arch::x86_64::{gdt, rdmsr, wrmsr};

const IA32_EFER: u32 = 0xC000_0080;
const IA32_STAR: u32 = 0xC000_0081;
const IA32_LSTAR: u32 = 0xC000_0082;
const IA32_FMASK: u32 = 0xC000_0084;

const EFER_SCE: u64 = 1 << 0;

static SYSCALL_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static USER_RSP_SCRATCH: AtomicU64 = AtomicU64::new(0);

pub fn set_kernel_stack(top: VirtAddr) {
    SYSCALL_STACK_TOP.store(top.as_u64(), Ordering::Relaxed);
}

/// Program the SYSCALL MSRs on the BSP.
///
/// # Safety
/// GDT must be installed; its selector layout is baked into `STAR`.
pub unsafe fn init_bsp() {
    let sel = gdt::selectors();
    // STAR[31:0] reserved. STAR[47:32] = kernel CS (SYSCALL uses CS and CS+8).
    // STAR[63:48] = user base. SYSRET uses user_base+16 for CS and user_base+8
    // for SS, so `user_base` must refer to user_DATA in the GDT.
    let kernel_cs = sel.kernel_code.0 as u64;
    let user_base = sel.user_data.0 as u64 - 8; // subtract ring bits so the
                                                // +8/+16 offsets land on user_data (ring 3) and user_code (ring 3).

    unsafe {
        wrmsr(IA32_EFER, rdmsr(IA32_EFER) | EFER_SCE);
        wrmsr(IA32_STAR, (kernel_cs << 32) | (user_base << 48));
        wrmsr(IA32_LSTAR, syscall_entry as *const () as u64);
        // Mask IF, DF, TF, NT, AC during kernel execution.
        wrmsr(IA32_FMASK, 0x4_0700);
    }
}

/// Assembly entry for `syscall`.
///
/// # Safety
/// Only invoked by the CPU via `syscall`. Assumes a valid current task and
/// that the scheduler has published its kernel stack top.
#[unsafe(naked)]
#[no_mangle]
pub unsafe extern "C" fn syscall_entry() {
    naked_asm!(
        // RSP is still the userspace stack here. Move to the current task's
        // kernel stack before touching user memory so syscalls survive CR3 switches.
        "mov qword ptr [rip + {user_rsp}], rsp",
        "mov rsp, qword ptr [rip + {stack_top}]",
        "and rsp, -16",
        "push qword ptr [rip + {user_rsp}]",
        // Stash user RCX (return RIP) and R11 (flags) — the CPU put them
        // there for us, but we save them on the stack so the dispatcher can
        // clobber.
        "push rcx",
        "push r11",
        // Preserve every register the Linux-style x86_64 syscall ABI says
        // the kernel must leave untouched across a `syscall`: that is,
        // everything except rax (return value), rcx (saved RIP), r11 (saved
        // flags). In particular, rdi/rsi/rdx/r10/r8/r9 carry syscall args
        // and userspace reuses them after the call without reloading.
        "push rdi",
        "push rsi",
        "push rdx",
        "push r10",
        "push r8",
        "push r9",
        // Callee-saved ring-0 also spills these.
        "push rbx",
        "push rbp",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        // Our dispatcher takes (a0, a1, a2, a3, a4, a5, num). Move r10→rcx
        // (a3) — rdi/rsi/rdx/r8/r9 are already in place. Num goes on the
        // stack as the 7th arg.
        "mov rcx, r10",
        "push rax",
        "call {dispatch}",
        "add rsp, 8",
        // Return value in rax. Unwind the saved context in reverse.
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbp",
        "pop rbx",
        "pop r9",
        "pop r8",
        "pop r10",
        "pop rdx",
        "pop rsi",
        "pop rdi",
        "pop r11",
        "pop rcx",
        "pop rsp",
        "sysretq",
        dispatch = sym crate::syscall::dispatch,
        stack_top = sym SYSCALL_STACK_TOP,
        user_rsp = sym USER_RSP_SCRATCH,
    );
}
