//! Ports — the receive side of an IPC channel.
//!
//! A port owns a bounded FIFO of in-flight messages. Senders push; receivers
//! pop (and block if empty). Messages own their transferred physical frames:
//! dropping a message that never reached its receiver frees those frames, so
//! the page-steal path can never leak on port teardown.

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;

use spin::Mutex;

use super::cap::Cap;
use crate::mm::pmm;

/// Inline control payload — 6 u64s ride in registers at send/recv.
pub const REG_SLOTS: usize = 6;
/// Per-port soft queue cap. Past this, senders see -EAGAIN.
pub const MAX_IN_FLIGHT: usize = 16;
/// Maximum caps transferred in one message.
pub const MAX_CAPS: usize = 8;

pub struct Message {
    pub regs: [u64; REG_SLOTS],
    pub caps: Vec<Arc<Cap>>,
    /// Physical frames owned by this message, detached from the sender's
    /// address space. Freed in `Drop` if never delivered.
    pub pages: Vec<u64>,
    /// Byte length of the page payload. `pages.len() * 4096 >= len`.
    pub len: usize,
}

impl Drop for Message {
    fn drop(&mut self) {
        for phys in self.pages.drain(..) {
            pmm::free_frame(phys);
        }
    }
}

pub struct Port {
    queue: Mutex<VecDeque<Message>>,
}

impl Port {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            queue: Mutex::new(VecDeque::new()),
        })
    }

    /// Pointer value used as the wait-queue key. The `Arc<Port>` keeps the
    /// allocation alive for as long as any receiver or message references it,
    /// so the address is a stable, unique token for `sched::block_on`.
    pub fn wait_key(self: &Arc<Self>) -> usize {
        Arc::as_ptr(self) as usize
    }

    /// Push a message. Returns `Err(msg)` giving the message back if the
    /// queue is full — caller drops it, which frees any stolen frames.
    pub fn try_push(&self, msg: Message) -> Result<(), Message> {
        let mut q = self.queue.lock();
        if q.len() >= MAX_IN_FLIGHT {
            return Err(msg);
        }
        q.push_back(msg);
        Ok(())
    }

    pub fn pop(&self) -> Option<Message> {
        self.queue.lock().pop_front()
    }
}
