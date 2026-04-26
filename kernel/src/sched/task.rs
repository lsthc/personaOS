//! Task objects — the unit the scheduler owns.
//!
//! A task has:
//!  - a 16 KiB kernel stack, allocated out of the kernel heap,
//!  - a save slot for RSP (the switch routine spills callee-saved there),
//!  - optionally, a user address space,
//!  - a state (Ready / Running / Dead).
//!
//! For freshly spawned user tasks we pre-build the kernel stack so that the
//! first `ret` the switcher performs lands in [`enter_user`], which in turn
//! `iretq`s into ring 3 using an IRETQ frame we stashed below it.

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::arch::naked_asm;
use core::cell::UnsafeCell;
use core::ptr;
use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};

use spin::Mutex;

use crate::arch::x86_64::gdt;
use crate::fs::{FdTable, OpenFile, SerialStdout, FD_READ, FD_WRITE};
use crate::mm::vmm::AddressSpace;

pub type TaskId = u64;

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TaskState {
    Ready = 0,
    Running = 1,
    Dead = 2,
    /// Removed from the run queue, parked on a wait-queue keyed by some
    /// pointer-sized token. Woken by `sched::wake_all`.
    Blocked = 3,
}

pub struct Task {
    id: TaskId,
    kstack: Box<[u8]>,
    saved_rsp: UnsafeCell<u64>,
    state: AtomicU8,
    addr_space: Mutex<Option<AddressSpace>>,
    fds: Mutex<FdTable>,
}

// SAFETY: `saved_rsp` is touched only by the scheduler with the run-queue
// lock held (serialised by context-switch protocol).
unsafe impl Send for Task {}
unsafe impl Sync for Task {}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

const KSTACK_SIZE: usize = 16 * 1024;

impl Task {
    pub fn id(&self) -> TaskId {
        self.id
    }
    pub fn state(&self) -> TaskState {
        match self.state.load(Ordering::Relaxed) {
            0 => TaskState::Ready,
            1 => TaskState::Running,
            3 => TaskState::Blocked,
            _ => TaskState::Dead,
        }
    }
    pub fn set_state(&self, s: TaskState) {
        self.state.store(s as u8, Ordering::Relaxed);
    }

    pub fn saved_rsp(&self) -> u64 {
        unsafe { *self.saved_rsp.get() }
    }
    pub fn saved_rsp_ptr(&self) -> *mut u64 {
        self.saved_rsp.get()
    }
    pub fn kstack_top(&self) -> u64 {
        let base = self.kstack.as_ptr() as u64;
        base + KSTACK_SIZE as u64
    }
    pub fn addr_space(&self) -> &Mutex<Option<AddressSpace>> {
        &self.addr_space
    }
    pub fn fds(&self) -> &Mutex<FdTable> {
        &self.fds
    }
}

fn new_id() -> TaskId {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

fn new_kstack() -> Box<[u8]> {
    let mut v = alloc::vec![0u8; KSTACK_SIZE].into_boxed_slice();
    // Zero already; returning.
    v.fill(0);
    v
}

/// Spawn a kernel thread entered at `entry` with no arguments. Used for the
/// idle task.
pub fn spawn_idle(entry: extern "C" fn() -> !) -> Arc<Task> {
    let kstack = new_kstack();
    let top = kstack.as_ptr() as u64 + KSTACK_SIZE as u64;

    // Build a "just been context-switched out" stack so switch_context can
    // resume it: [r15 r14 r13 r12 rbp rbx ret_addr]
    let rsp = unsafe {
        let mut sp = top;
        sp -= 8;
        ptr::write(sp as *mut u64, entry as usize as u64); // return addr for `ret`
        // callee-saved (all zero)
        for _ in 0..6 {
            sp -= 8;
            ptr::write(sp as *mut u64, 0);
        }
        sp
    };

    Arc::new(Task {
        id: new_id(),
        kstack,
        saved_rsp: UnsafeCell::new(rsp),
        state: AtomicU8::new(TaskState::Ready as u8),
        addr_space: Mutex::new(None),
        fds: Mutex::new(FdTable::new()),
    })
}

/// Spawn a user task. `entry_user` and `stack_user` are already mapped in
/// `address_space`.
pub fn spawn_user(
    address_space: AddressSpace,
    entry_user: u64,
    stack_user: u64,
) -> Arc<Task> {
    let sel = gdt::selectors();
    let user_cs = sel.user_code.0 as u64 | 3;
    let user_ss = sel.user_data.0 as u64 | 3;
    // IF=1 (interrupts enabled in user mode), reserved bit 1 set.
    let user_rflags = 0x202u64;

    let kstack = new_kstack();
    let top = kstack.as_ptr() as u64 + KSTACK_SIZE as u64;

    let rsp = unsafe {
        let mut sp = top;
        // IRETQ frame, pushed in reverse pop order (SS, RSP, RFLAGS, CS, RIP).
        sp -= 8; ptr::write(sp as *mut u64, user_ss);
        sp -= 8; ptr::write(sp as *mut u64, stack_user);
        sp -= 8; ptr::write(sp as *mut u64, user_rflags);
        sp -= 8; ptr::write(sp as *mut u64, user_cs);
        sp -= 8; ptr::write(sp as *mut u64, entry_user);
        // Return address for the switch routine's final `ret`: enter_user.
        sp -= 8; ptr::write(sp as *mut u64, enter_user as *const () as u64);
        // Callee-saved registers (zeroed).
        for _ in 0..6 {
            sp -= 8;
            ptr::write(sp as *mut u64, 0);
        }
        sp
    };

    let mut fds = FdTable::new();
    let stdout = OpenFile::new(Arc::new(SerialStdout) as _, FD_WRITE);
    let stdin = OpenFile::new(Arc::new(SerialStdout) as _, FD_READ);
    fds.install_at(0, stdin);
    fds.install_at(1, stdout.clone());
    fds.install_at(2, stdout);

    Arc::new(Task {
        id: new_id(),
        kstack,
        saved_rsp: UnsafeCell::new(rsp),
        state: AtomicU8::new(TaskState::Ready as u8),
        addr_space: Mutex::new(Some(address_space)),
        fds: Mutex::new(fds),
    })
}

/// Transition to user mode using the IRETQ frame sitting directly above RSP.
/// The switch routine arranges for this function to be the return target of
/// its final `ret`, with RSP pointing at the frame.
#[unsafe(naked)]
unsafe extern "C" fn enter_user() {
    naked_asm!(
        "iretq",
    );
}
