//! IPC — ports, capabilities, zero-copy message passing.
//!
//! The kernel-side of personaOS's userspace-services ABI. Userspace creates
//! ports, mints capabilities, and exchanges messages whose bulk payloads are
//! *page-stolen* from the sender's address space into the receiver's — no
//! bulk copy in the kernel.
//!
//! See `/root/.claude/plans/snappy-coalescing-minsky.md` for the design.

pub mod cap;
pub mod cap_table;
pub mod port;
pub mod registry;
pub mod syscalls;

pub use cap::Cap;
#[allow(unused_imports)] // consumed by M3.2+ services
pub use cap::{CapObject, Rights};
#[allow(unused_imports)]
pub use cap_table::CapId;
pub use cap_table::CapTable;
#[allow(unused_imports)]
pub use port::Port;

/// Negative errno values returned over the syscall ABI.
#[allow(dead_code)]
pub mod errno {
    pub const EPERM: i64 = -1;
    pub const ENOENT: i64 = -2;
    pub const EFAULT: i64 = -14;
    pub const EINVAL: i64 = -22;
    pub const EAGAIN: i64 = -11;
    pub const ENOMEM: i64 = -12;
    pub const EEXIST: i64 = -17;
    pub const EBADF: i64 = -9;
    pub const E2BIG: i64 = -7;
}
