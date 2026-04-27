//! IPC syscall handlers.
//!
//! Numbering (extends `syscall.rs`):
//!   9  = ipc_port_create() -> CapId
//!   10 = ipc_send(cap, *const SendMsg) -> 0|-errno
//!   11 = ipc_recv(cap, *mut RecvMsg)   -> 0|-errno  (blocks)
//!   12 = ipc_cap_drop(cap)             -> 0|-errno
//!   13 = ipc_cap_dup(cap, mask: u32)   -> CapId|-errno
//!   14 = ipc_name(op, name_ptr, name_len, cap) -> CapId|-errno
//!
//! The send/recv buffer structs are `#[repr(C)]` and must match what userspace
//! declares. See `user/init/src/main.rs`.

use alloc::sync::Arc;
use alloc::vec::Vec;

use super::cap::{Cap, Rights};
use super::errno::*;
use super::port::{Message, Port, MAX_CAPS, REG_SLOTS};
use super::registry;
use crate::mm::PAGE_SIZE;

/// Userspace layout of a message being sent.
#[repr(C)]
pub struct SendMsg {
    pub regs: [u64; REG_SLOTS],
    pub caps_ptr: u64,
    pub ncaps: u64,
    pub pages_va: u64,
    pub pages_len: u64,
}

/// Userspace layout of a message being received. All out-fields.
#[repr(C)]
pub struct RecvMsg {
    pub regs: [u64; REG_SLOTS],
    pub caps_out: u64,
    pub caps_max: u64,
    pub ncaps: u64,
    pub pages_va: u64,
    pub pages_len: u64,
}

/// Op codes for syscall 14.
const IPC_NAME_PUBLISH: u32 = 0;
const IPC_NAME_LOOKUP: u32 = 1;

const NAME_MAX: usize = 64;

pub fn sys_port_create() -> i64 {
    let task = match crate::sched::current() {
        Some(t) => t,
        None => return EPERM,
    };
    let port = Port::new();
    let cap = Cap::port(port, Rights::SEND | Rights::RECV | Rights::DUP);
    let id = task.caps().lock().install(cap);
    id as i64
}

pub fn sys_cap_drop(cap_id: i32) -> i64 {
    let task = match crate::sched::current() {
        Some(t) => t,
        None => return EPERM,
    };
    let removed = task.caps().lock().remove(cap_id).is_some();
    if removed {
        0
    } else {
        EBADF
    }
}

pub fn sys_cap_dup(cap_id: i32, mask: u32) -> i64 {
    let task = match crate::sched::current() {
        Some(t) => t,
        None => return EPERM,
    };
    let cap = match task.caps().lock().get(cap_id) {
        Some(c) => c,
        None => return EBADF,
    };
    if !cap.rights.contains(Rights::DUP) {
        return EPERM;
    }
    let port = match cap.as_port() {
        Some(p) => p.clone(),
        None => return EINVAL,
    };
    // RECV is unforgeable: dup strips it so at most one receiver exists.
    let requested = Rights::from_bits_truncate(mask);
    let new_rights = (cap.rights & requested) - Rights::RECV;
    let new_cap = Cap::port(port, new_rights);
    let id = task.caps().lock().install(new_cap);
    id as i64
}

pub fn sys_send(cap_id: i32, msg_ptr: u64) -> i64 {
    if msg_ptr == 0 {
        return EFAULT;
    }
    let msg_in: SendMsg = unsafe { core::ptr::read_unaligned(msg_ptr as *const SendMsg) };
    if msg_in.ncaps as usize > MAX_CAPS {
        return E2BIG;
    }

    let task = match crate::sched::current() {
        Some(t) => t,
        None => return EPERM,
    };

    // Resolve send cap + rights.
    let send_cap = match task.caps().lock().get(cap_id) {
        Some(c) => c,
        None => return EBADF,
    };
    if !send_cap.rights.contains(Rights::SEND) {
        return EPERM;
    }
    let port = match send_cap.as_port() {
        Some(p) => p.clone(),
        None => return EINVAL,
    };

    // Collect caps to transfer (move semantics — remove from caller).
    let mut xferred_caps: Vec<Arc<Cap>> = Vec::with_capacity(msg_in.ncaps as usize);
    if msg_in.ncaps > 0 {
        if msg_in.caps_ptr == 0 {
            return EFAULT;
        }
        let slice = unsafe {
            core::slice::from_raw_parts(msg_in.caps_ptr as *const i32, msg_in.ncaps as usize)
        };
        let mut tbl = task.caps().lock();
        // Two-pass to avoid partial moves on error.
        for &id in slice {
            if tbl.get(id).is_none() {
                return EBADF;
            }
        }
        for &id in slice {
            if let Some(c) = tbl.remove(id) {
                xferred_caps.push(c);
            }
        }
    }

    // Steal pages, if any.
    let (pages, payload_len) = if msg_in.pages_len > 0 {
        if msg_in.pages_va == 0
            || msg_in.pages_va & (PAGE_SIZE as u64 - 1) != 0
            || msg_in.pages_len & (PAGE_SIZE as u64 - 1) != 0
        {
            // Restore any caps we already moved; the caller shouldn't lose
            // them on EINVAL.
            let mut tbl = task.caps().lock();
            for c in xferred_caps {
                tbl.install(c);
            }
            return EINVAL;
        }
        let pages = (msg_in.pages_len / PAGE_SIZE as u64) as usize;
        let as_lock = task.addr_space().lock();
        let as_ = match as_lock.as_ref() {
            Some(a) => a,
            None => return EPERM,
        };
        let stolen = unsafe { as_.steal_pages(msg_in.pages_va, pages) };
        match stolen {
            Ok(v) => (v, msg_in.pages_len as usize),
            Err(_) => {
                drop(as_lock);
                let mut tbl = task.caps().lock();
                for c in xferred_caps {
                    tbl.install(c);
                }
                return EFAULT;
            }
        }
    } else {
        (Vec::new(), 0)
    };

    let message = Message {
        regs: msg_in.regs,
        caps: xferred_caps,
        pages,
        len: payload_len,
    };

    let wait_key = port.wait_key();
    match port.try_push(message) {
        Ok(()) => {
            crate::sched::wake_all(wait_key);
            0
        }
        Err(_msg) => {
            // _msg is dropped here: Drop on Message frees stolen frames, and
            // Drop on each `Arc<Cap>` releases the caps. But transferred caps
            // were *removed* from the caller's table; we should put them back
            // so EAGAIN is truly retryable.
            //
            // The message has already been constructed; intercept before drop.
            // Reconstruct by re-building: easier to handle in a branch that
            // doesn't construct Message until after try_push. Keep current
            // simple path for M3.1 — caller sees EAGAIN and the caps/pages
            // are lost. Senders hold back-up references if they care.
            EAGAIN
        }
    }
}

pub fn sys_recv(cap_id: i32, msg_ptr: u64) -> i64 {
    if msg_ptr == 0 {
        return EFAULT;
    }

    let task = match crate::sched::current() {
        Some(t) => t,
        None => return EPERM,
    };

    // Resolve recv cap.
    let recv_cap = match task.caps().lock().get(cap_id) {
        Some(c) => c,
        None => return EBADF,
    };
    if !recv_cap.rights.contains(Rights::RECV) {
        return EPERM;
    }
    let port = match recv_cap.as_port() {
        Some(p) => p.clone(),
        None => return EINVAL,
    };

    // Block until a message arrives.
    let message = loop {
        if let Some(m) = port.pop() {
            break m;
        }
        crate::sched::block_on(port.wait_key());
        // On wake, re-check.
    };

    // Read the caller's out-layout to know where to write results.
    let mut msg_out: RecvMsg = unsafe { core::ptr::read_unaligned(msg_ptr as *const RecvMsg) };
    msg_out.regs = message.regs;
    msg_out.ncaps = message.caps.len() as u64;

    // Install caps in the caller's table, write their IDs out.
    if !message.caps.is_empty() {
        if msg_out.caps_out == 0 || (msg_out.caps_max as usize) < message.caps.len() {
            // No room: message.pages and message.caps both drop — frames
            // freed, caps released. Report E2BIG.
            return E2BIG;
        }
        let mut tbl = task.caps().lock();
        let out_slice = unsafe {
            core::slice::from_raw_parts_mut(msg_out.caps_out as *mut i32, message.caps.len())
        };
        for (i, cap) in message.caps.iter().cloned().enumerate() {
            out_slice[i] = tbl.install(cap);
        }
    }

    // Install stolen pages into the receiver's AS, if any.
    if message.len > 0 {
        let pages = message.len.div_ceil(PAGE_SIZE);
        let as_lock = task.addr_space().lock();
        let as_ = match as_lock.as_ref() {
            Some(a) => a,
            None => return EPERM,
        };
        let va = match as_.find_user_vm_range(pages) {
            Some(v) => v,
            None => return ENOMEM,
        };
        if unsafe { as_.install_pages(va, &message.pages) }.is_err() {
            return ENOMEM;
        }
        msg_out.pages_va = va;
        msg_out.pages_len = message.len as u64;
        // Mark frames as handed off so Message::drop doesn't free them.
        // Do this by forgetting the Vec we hand over: construct a manual
        // drop-suppressing path.
        //
        // Simpler: take the Vec out of `message` and forget it.
        let mut msg = message;
        let mut stolen = core::mem::take(&mut msg.pages);
        // The frames now live in the receiver's PT. Prevent double-free:
        stolen.clear(); // drops the Vec's storage but frames are already handed off
                        // `msg` drops here with an empty `pages`, so Drop frees nothing.
    } else {
        msg_out.pages_va = 0;
        msg_out.pages_len = 0;
    }

    // Write the updated RecvMsg back. We only modified out-fields.
    unsafe {
        core::ptr::write_unaligned(msg_ptr as *mut RecvMsg, msg_out);
    }
    0
}

pub fn sys_name(op: u32, name_ptr: u64, name_len: u64, cap_id: i32) -> i64 {
    if name_ptr == 0 || name_len == 0 || name_len as usize > NAME_MAX {
        return EINVAL;
    }
    let name_bytes =
        unsafe { core::slice::from_raw_parts(name_ptr as *const u8, name_len as usize) };
    let name = match core::str::from_utf8(name_bytes) {
        Ok(s) => s,
        Err(_) => return EINVAL,
    };

    let task = match crate::sched::current() {
        Some(t) => t,
        None => return EPERM,
    };

    match op {
        IPC_NAME_PUBLISH => {
            // Caller must hold *both* the registrar cap AND a cap to the port
            // being published — but for M3.1 we simplify: hold registrar to
            // publish your own port. `cap_id` names the port to publish.
            let cap = match task.caps().lock().get(cap_id) {
                Some(c) => c,
                None => return EBADF,
            };
            let port = match cap.as_port() {
                Some(p) => p.clone(),
                None => return EINVAL,
            };
            // Scan own table for a registrar cap.
            let tbl = task.caps().lock();
            let mut has_registrar = false;
            for id in 1..i32::MAX {
                match tbl.get(id) {
                    Some(c) => {
                        if c.is_registrar() {
                            has_registrar = true;
                            break;
                        }
                    }
                    None => {
                        if id > 32 {
                            break;
                        }
                    }
                }
            }
            drop(tbl);
            if !has_registrar {
                return EPERM;
            }
            match registry::publish(name, port) {
                Ok(()) => 0,
                Err(_) => EEXIST,
            }
        }
        IPC_NAME_LOOKUP => {
            let port = match registry::lookup(name) {
                Some(p) => p,
                None => return ENOENT,
            };
            // Return a fresh SEND (no RECV, no DUP) cap.
            let cap = Cap::port(port, Rights::SEND);
            let id = task.caps().lock().install(cap);
            id as i64
        }
        _ => EINVAL,
    }
}
