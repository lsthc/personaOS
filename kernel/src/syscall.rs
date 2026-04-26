//! Syscall dispatcher, wired up to the `syscall`/`sysret` MSRs in `arch`.
//!
//! Numbering (stable for M2):
//!   0 = write(fd, buf, len)
//!   1 = exit(code)
//!   2 = yield()
//!   3 = getpid()
//!   4 = open(path_ptr, path_len, flags) -> fd
//!   5 = read(fd, buf, len)
//!   6 = close(fd)
//!   7 = lseek(fd, offset, whence)
//!   8 = fstat(fd, stat_buf_ptr)
//!
//! Userspace sees `i64`-encoded return values: negative = -FsError variant
//! number (1..=N), non-negative = success.

use core::sync::atomic::Ordering;

use crate::fs;

// `a3`/`a4`/`a5` reserved for future calls (open mode, pread, etc.). Drop
// the unused-variable noise without pretending they're load-bearing.
#[allow(dead_code)]
const _: () = ();

const SYS_WRITE: u64 = 0;
const SYS_EXIT: u64 = 1;
const SYS_YIELD: u64 = 2;
const SYS_GETPID: u64 = 3;
const SYS_OPEN: u64 = 4;
const SYS_READ: u64 = 5;
const SYS_CLOSE: u64 = 6;
const SYS_LSEEK: u64 = 7;
const SYS_FSTAT: u64 = 8;

const SEEK_SET: u64 = 0;
const SEEK_CUR: u64 = 1;
const SEEK_END: u64 = 2;

const MAX_IO: usize = 4096;

/// C-ABI entry called by the `syscall` assembly stub in
/// `arch::x86_64::syscall`.
#[no_mangle]
pub extern "C" fn dispatch(a0: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64, num: u64) -> u64 {
    let _ = (a3, a4, a5);
    match num {
        SYS_WRITE => sys_write(a0 as i32, a1 as *const u8, a2 as usize) as u64,
        SYS_EXIT => crate::sched::current_exit(a0 as i32),
        SYS_YIELD => {
            crate::sched::yield_now();
            0
        }
        SYS_GETPID => crate::sched::current_id().unwrap_or(0),
        SYS_OPEN => sys_open(a0 as *const u8, a1 as usize, a2 as u32) as u64,
        SYS_READ => sys_read(a0 as i32, a1 as *mut u8, a2 as usize) as u64,
        SYS_CLOSE => sys_close(a0 as i32) as u64,
        SYS_LSEEK => sys_lseek(a0 as i32, a1 as i64, a2) as u64,
        SYS_FSTAT => sys_fstat(a0 as i32, a1 as *mut u8) as u64,
        _ => -1i64 as u64,
    }
}

fn fs_err_to_code(e: fs::FsError) -> i64 {
    -(e as i64 + 1)
}

fn sys_write(fd: i32, buf: *const u8, len: usize) -> i64 {
    if len > MAX_IO || buf.is_null() {
        return -1;
    }
    let bytes = unsafe { core::slice::from_raw_parts(buf, len) };
    let task = match crate::sched::current() {
        Some(t) => t,
        None => return -1,
    };
    let file = match task.fds().lock().get(fd) {
        Some(f) => f,
        None => return fs_err_to_code(fs::FsError::BadFd),
    };
    let off = file.offset.load(Ordering::Relaxed);
    match file.inode.write_at(off, bytes) {
        Ok(n) => {
            file.offset.fetch_add(n as u64, Ordering::Relaxed);
            n as i64
        }
        Err(e) => fs_err_to_code(e),
    }
}

fn sys_read(fd: i32, buf: *mut u8, len: usize) -> i64 {
    if len > MAX_IO || buf.is_null() {
        return -1;
    }
    let slice = unsafe { core::slice::from_raw_parts_mut(buf, len) };
    let task = match crate::sched::current() {
        Some(t) => t,
        None => return -1,
    };
    let file = match task.fds().lock().get(fd) {
        Some(f) => f,
        None => return fs_err_to_code(fs::FsError::BadFd),
    };
    let off = file.offset.load(Ordering::Relaxed);
    match file.inode.read_at(off, slice) {
        Ok(n) => {
            file.offset.fetch_add(n as u64, Ordering::Relaxed);
            n as i64
        }
        Err(e) => fs_err_to_code(e),
    }
}

fn sys_open(path_ptr: *const u8, path_len: usize, flags: u32) -> i64 {
    if path_len == 0 || path_len > 256 || path_ptr.is_null() {
        return -1;
    }
    let bytes = unsafe { core::slice::from_raw_parts(path_ptr, path_len) };
    let path = match core::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return fs_err_to_code(fs::FsError::InvalidPath),
    };
    let inode = match fs::lookup(path) {
        Ok(n) => n,
        Err(e) => {
            // Honour O_CREATE — bit 2, per fs::FD_CREATE.
            if flags & fs::FD_CREATE != 0 && e == fs::FsError::NotFound {
                let (parent, name) = match fs::split_parent(path) {
                    Some(p) => p,
                    None => return fs_err_to_code(fs::FsError::InvalidPath),
                };
                let pdir = match fs::lookup(parent) {
                    Ok(p) => p,
                    Err(e) => return fs_err_to_code(e),
                };
                match pdir.create(name, fs::InodeKind::File) {
                    Ok(n) => n,
                    Err(e) => return fs_err_to_code(e),
                }
            } else {
                return fs_err_to_code(e);
            }
        }
    };
    let open = fs::OpenFile::new(inode, flags);
    let task = match crate::sched::current() {
        Some(t) => t,
        None => return -1,
    };
    let fd = task.fds().lock().install(open);
    fd as i64
}

fn sys_close(fd: i32) -> i64 {
    let task = match crate::sched::current() {
        Some(t) => t,
        None => return -1,
    };
    if task.fds().lock().close(fd) { 0 } else { fs_err_to_code(fs::FsError::BadFd) }
}

fn sys_lseek(fd: i32, off: i64, whence: u64) -> i64 {
    let task = match crate::sched::current() {
        Some(t) => t,
        None => return -1,
    };
    let file = match task.fds().lock().get(fd) {
        Some(f) => f,
        None => return fs_err_to_code(fs::FsError::BadFd),
    };
    let new_off = match whence {
        SEEK_SET => off.max(0) as u64,
        SEEK_CUR => {
            let cur = file.offset.load(Ordering::Relaxed) as i64;
            (cur + off).max(0) as u64
        }
        SEEK_END => {
            let size = file.inode.size() as i64;
            (size + off).max(0) as u64
        }
        _ => return -1,
    };
    file.offset.store(new_off, Ordering::Relaxed);
    new_off as i64
}

// Userspace stat layout: [ size: u64, kind: u32, _pad: u32 ] (16 bytes).
fn sys_fstat(fd: i32, buf: *mut u8) -> i64 {
    if buf.is_null() {
        return -1;
    }
    let task = match crate::sched::current() {
        Some(t) => t,
        None => return -1,
    };
    let file = match task.fds().lock().get(fd) {
        Some(f) => f,
        None => return fs_err_to_code(fs::FsError::BadFd),
    };
    let size = file.inode.size();
    let kind: u32 = match file.inode.kind() {
        fs::InodeKind::File => 0,
        fs::InodeKind::Dir => 1,
    };
    unsafe {
        core::ptr::write_unaligned(buf as *mut u64, size);
        core::ptr::write_unaligned(buf.add(8) as *mut u32, kind);
        core::ptr::write_unaligned(buf.add(12) as *mut u32, 0);
    }
    0
}
