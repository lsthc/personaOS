//! Capabilities — unforgeable handles to kernel-managed objects.
//!
//! A `Cap` pairs an object reference with a set of `Rights`. Userspace holds
//! these by `CapId` (an integer slot in the task's cap table); the kernel side
//! is always reached through an `Arc<Cap>`. Rights are checked at every
//! syscall; the slot itself is the trust boundary, identical to how fds work.

use alloc::sync::Arc;
use bitflags::bitflags;

use super::port::Port;

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub struct Rights: u32 {
        /// Allowed to `ipc_send` through this cap.
        const SEND = 1 << 0;
        /// Allowed to `ipc_recv` through this cap. At most one live cap in the
        /// whole system carries this bit for a given port.
        const RECV = 1 << 1;
        /// Allowed to `ipc_cap_dup` this cap. Dup strips `RECV` unconditionally.
        const DUP  = 1 << 2;
    }
}

#[allow(dead_code)]
/// What a capability refers to. `Vm` is reserved for M4's framebuffer
/// hand-off; present now so the enum shape is stable across M3.
pub enum CapObject {
    Port(Arc<Port>),
    /// Placeholder for M4. Never constructed in M3.
    #[allow(dead_code)]
    Vm,
    /// Authorizes `ipc_name publish`. Minted once at boot, handed to PID 1.
    Registrar,
}

pub struct Cap {
    pub object: CapObject,
    pub rights: Rights,
}

impl Cap {
    pub fn port(p: Arc<Port>, rights: Rights) -> Arc<Self> {
        Arc::new(Self {
            object: CapObject::Port(p),
            rights,
        })
    }
    pub fn registrar() -> Arc<Self> {
        Arc::new(Self {
            object: CapObject::Registrar,
            rights: Rights::empty(),
        })
    }

    pub fn as_port(&self) -> Option<&Arc<Port>> {
        match &self.object {
            CapObject::Port(p) => Some(p),
            _ => None,
        }
    }
    pub fn is_registrar(&self) -> bool {
        matches!(self.object, CapObject::Registrar)
    }
}
