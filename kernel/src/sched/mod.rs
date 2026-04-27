//! Round-robin preemptive scheduler with a single BSP run queue.
//!
//! Tasks are scheduled by the LAPIC timer tick. Each task owns a kernel
//! stack and, optionally, a user address space. Context switches save
//! callee-saved registers onto the outgoing task's kernel stack, swap CR3
//! if the next task lives in a different address space, update TSS.RSP0
//! so the CPU knows where to land on the next ring-3→ring-0 transition,
//! and resume the incoming task.

mod task;

pub use task::{spawn_idle, spawn_user, Task, TaskId, TaskState};

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::arch::{asm, naked_asm};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use spin::Mutex;

/// Opaque key identifying what a task is blocked on. Any pointer-sized value
/// the driver chooses — typically the address of the shared structure
/// (completion queue, buffer) whose state change will wake the sleeper.
pub type WaitKey = usize;

use crate::arch::x86_64::gdt;

static RUN_QUEUE: Mutex<Vec<Arc<Task>>> = Mutex::new(Vec::new());
static CURRENT: Mutex<Option<Arc<Task>>> = Mutex::new(None);
static TASKS: Mutex<Vec<Arc<Task>>> = Mutex::new(Vec::new());
static TICKS: AtomicUsize = AtomicUsize::new(0);
/// Set from any IRQ context that wants to force a reschedule; cleared by
/// the scheduler itself.
static NEED_RESCHED: AtomicBool = AtomicBool::new(false);

/// Parked tasks keyed by a caller-chosen pointer-sized value. All waiters for
/// a given key are woken together — that's the only primitive we need for M2
/// (one waiter per completion queue or dirtied buffer).
static WAIT_QUEUES: Mutex<Vec<(WaitKey, Arc<Task>)>> = Mutex::new(Vec::new());

pub fn enqueue(task: Arc<Task>) {
    RUN_QUEUE.lock().push(task);
}

pub fn register(task: &Arc<Task>) {
    TASKS.lock().push(task.clone());
}

pub fn find_task(pid: TaskId) -> Option<Arc<Task>> {
    TASKS.lock().iter().find(|t| t.id() == pid).cloned()
}

pub fn is_child(parent: TaskId, child: TaskId) -> bool {
    find_task(child).is_some_and(|t| t.parent() == parent)
}

#[allow(dead_code)]
pub fn ticks() -> usize {
    TICKS.load(Ordering::Relaxed)
}

/// Called from the LAPIC timer handler. Drives preemption directly: every
/// timer tick triggers a context-switch candidate. The switch routine is a
/// no-op when the run queue has no other task ready to run, so the idle
/// path is just a loop of tick → schedule → same task.
pub fn on_tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
    schedule();
}

/// Voluntarily give up the CPU. Used by the `yield` syscall.
pub fn yield_now() {
    schedule();
}

/// Run the scheduler loop. Must be invoked once on the BSP with at least
/// one task enqueued; does not return.
pub fn run() -> ! {
    // Pick the first task, install it as current, then jump into it.
    let first = {
        let mut q = RUN_QUEUE.lock();
        q.remove(0)
    };
    *CURRENT.lock() = Some(first.clone());
    unsafe {
        prepare_to_run(&first);
        // Load the entry context. For the very first invocation, RSP is
        // pointing at the initial frame we built in `task::new_*`.
        let rsp = first.saved_rsp();
        asm!(
            "mov rsp, {rsp}",
            "pop r15",
            "pop r14",
            "pop r13",
            "pop r12",
            "pop rbp",
            "pop rbx",
            "ret",
            rsp = in(reg) rsp,
            options(noreturn),
        );
    }
}

/// The meat of the scheduler: pick the next task, switch to it. Safe to
/// call from IRQ or syscall context.
pub fn schedule() {
    NEED_RESCHED.store(false, Ordering::Relaxed);

    let (prev, next) = {
        let mut q = RUN_QUEUE.lock();
        let mut cur_slot = CURRENT.lock();
        let prev = cur_slot.clone();
        let next = match q.first().cloned() {
            Some(t) => {
                q.remove(0);
                t
            }
            None => return, // nothing to switch to
        };
        if let Some(ref p) = prev {
            if p.state() == TaskState::Ready || p.state() == TaskState::Running {
                p.set_state(TaskState::Ready);
                q.push(p.clone());
            }
        }
        next.set_state(TaskState::Running);
        *cur_slot = Some(next.clone());
        (prev, next)
    };

    if let Some(prev) = prev {
        if Arc::ptr_eq(&prev, &next) {
            return;
        }
        unsafe {
            prepare_to_run(&next);
            switch_context(prev.saved_rsp_ptr(), next.saved_rsp());
        }
    } else {
        // Should only happen during the bootstrap path.
        unsafe {
            prepare_to_run(&next);
            let rsp = next.saved_rsp();
            asm!(
                "mov rsp, {rsp}",
                "pop r15",
                "pop r14",
                "pop r13",
                "pop r12",
                "pop rbp",
                "pop rbx",
                "ret",
                rsp = in(reg) rsp,
                options(noreturn),
            );
        }
    }
}

unsafe fn prepare_to_run(next: &Task) {
    if let Some(ref as_) = *next.addr_space().lock() {
        unsafe {
            as_.activate();
        }
    }
    let top = x86_64::VirtAddr::new(next.kstack_top());
    gdt::set_kernel_stack(top);
    crate::arch::x86_64::syscall::set_kernel_stack(top);
}

/// Save the outgoing task's context and load the incoming one. The trick:
/// both tasks look identical from the CPU's point of view at the moment of
/// the swap — each is suspended between the `push`es and the `ret` below,
/// so restoring the new RSP and `ret`ing just continues whatever the next
/// task was doing.
#[unsafe(naked)]
unsafe extern "C" fn switch_context(prev_rsp: *mut u64, next_rsp: u64) {
    naked_asm!(
        "push rbx",
        "push rbp",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov [rdi], rsp",
        "mov rsp, rsi",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbp",
        "pop rbx",
        "ret",
    );
}

/// Called from interrupt return paths. If preemption was requested, switch.
#[allow(dead_code)]
pub fn preempt_if_needed() {
    if NEED_RESCHED.load(Ordering::Relaxed) {
        schedule();
    }
}

/// Mark the current task as dead and never schedule it again.
pub fn current_exit(code: i32) -> ! {
    {
        let cur = CURRENT.lock();
        if let Some(ref t) = *cur {
            t.set_exit_code(code);
            t.set_state(TaskState::Dead);
            wake_all(wait_key(t.parent()));
        }
    }
    // Drop current so `schedule` picks a new task.
    *CURRENT.lock() = None;
    schedule();
    // If schedule returned, nothing else to run — just halt forever.
    loop {
        crate::arch::x86_64::halt();
    }
}

/// Current task's id (for debugging / syscalls like getpid).
pub fn current_id() -> Option<TaskId> {
    CURRENT.lock().as_ref().map(|t| t.id())
}

/// Arc to the currently running task. Used by syscall handlers that need to
/// poke at per-task state (FD table, address space).
pub fn current() -> Option<Arc<Task>> {
    CURRENT.lock().clone()
}

pub fn wait_key(parent_pid: TaskId) -> WaitKey {
    0x5750_0000_0000_0000usize | parent_pid as usize
}

pub fn find_exited_child(parent_pid: TaskId, pid: TaskId) -> Option<(TaskId, i32)> {
    TASKS
        .lock()
        .iter()
        .find(|t| {
            t.parent() == parent_pid && (pid == 0 || t.id() == pid) && t.state() == TaskState::Dead
        })
        .map(|t| (t.id(), t.exit_code()))
}

pub fn has_child(parent_pid: TaskId, pid: TaskId) -> bool {
    TASKS
        .lock()
        .iter()
        .any(|t| t.parent() == parent_pid && (pid == 0 || t.id() == pid))
}

pub fn kill(pid: TaskId, code: i32) -> bool {
    let target = match find_task(pid) {
        Some(t) => t,
        None => return false,
    };
    target.set_exit_code(code);
    target.set_state(TaskState::Dead);
    {
        let mut rq = RUN_QUEUE.lock();
        let mut i = 0;
        while i < rq.len() {
            if rq[i].id() == pid {
                rq.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }
    wake_all(wait_key(target.parent()));
    true
}

/// Park the current task on a wait-queue keyed by `key` and yield the CPU.
/// The caller must ensure that whatever produces the wake-up (typically a
/// driver completion IRQ) will call [`wake_all`] with the same key.
///
/// Interrupts should be disabled at the call site while the wake condition
/// is being checked to avoid missing an IRQ that lands between the check and
/// the park — the driver pattern is: `cli; if !done { block_on(key); }`.
#[allow(dead_code)] // consumed by NVMe / xHCI completion waits
pub fn block_on(key: WaitKey) {
    let cur = CURRENT.lock().as_ref().cloned();
    if let Some(t) = cur {
        t.set_state(TaskState::Blocked);
        WAIT_QUEUES.lock().push((key, t));
    }
    schedule();
}

/// Wake every task blocked on `key`. Safe from IRQ context (takes a spin
/// lock, no allocation). Woken tasks land back on the run queue in Ready
/// state; the next `schedule()` will pick them up.
#[allow(dead_code)] // consumed by NVMe / xHCI completion IRQs
pub fn wake_all(key: WaitKey) {
    let mut woken: Vec<Arc<Task>> = Vec::new();
    {
        let mut wq = WAIT_QUEUES.lock();
        let mut i = 0;
        while i < wq.len() {
            if wq[i].0 == key {
                let (_, t) = wq.swap_remove(i);
                woken.push(t);
            } else {
                i += 1;
            }
        }
    }
    if !woken.is_empty() {
        let mut rq = RUN_QUEUE.lock();
        for t in woken {
            t.set_state(TaskState::Ready);
            rq.push(t);
        }
    }
}
