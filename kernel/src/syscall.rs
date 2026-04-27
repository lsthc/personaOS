//! Syscall dispatcher, wired up to the `syscall`/`sysret` MSRs in `arch`.
//!
//! Numbering (stable for M4):
//!   0 = write(fd, buf, len)
//!   1 = exit(code)
//!   2 = yield()
//!   3 = getpid()
//!   4 = open(path_ptr, path_len, flags) -> fd
//!   5 = read(fd, buf, len)
//!   6 = close(fd)
//!   7 = lseek(fd, offset, whence)
//!   8 = fstat(fd, stat_buf_ptr)
//!   9 = ipc_port_create() -> cap_id
//!  10 = ipc_send(cap_id, *const SendMsg) -> 0|-errno
//!  11 = ipc_recv(cap_id, *mut RecvMsg) -> 0|-errno (blocks)
//!  12 = ipc_cap_drop(cap_id) -> 0|-errno
//!  13 = ipc_cap_dup(cap_id, mask) -> new_cap_id|-errno
//!  14 = ipc_name(op, name_ptr, name_len, cap_id) -> 0|cap_id|-errno
//!  15 = spawn(path_ptr, path_len, flags) -> pid|-errno
//!  16 = waitpid(pid, status_ptr, flags) -> pid|0|-errno
//!  17 = kill(pid, code) -> 0|-errno
//!  18 = display_info(info_ptr) -> 0|-errno
//!  19 = display_clear(rgb) -> 0|-errno
//!  20 = display_fill_rect(x, y, w, h, rgb) -> 0|-errno
//!  21 = display_draw_text(x, y, buf, len, rgb) -> bytes|-errno
//!  22 = input_poll_byte() -> byte|0
//!  23 = net_configure(info_ptr) -> 0|-errno
//!  24 = net_info(info_ptr) -> 0|-errno
//!  25 = audio_play_tone(freq_hz, duration_ms, info_ptr) -> 0|-errno
//!  26 = audio_info(info_ptr) -> 0|-errno
//!
//! Userspace sees `i64`-encoded return values: negative = -errno (or
//! -FsError variant for fs calls), non-negative = success.

use core::sync::atomic::Ordering;

use crate::fs;
use crate::ipc::syscalls as ipc_sys;

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
const SYS_IPC_PORT_CREATE: u64 = 9;
const SYS_IPC_SEND: u64 = 10;
const SYS_IPC_RECV: u64 = 11;
const SYS_IPC_CAP_DROP: u64 = 12;
const SYS_IPC_CAP_DUP: u64 = 13;
const SYS_IPC_NAME: u64 = 14;
const SYS_SPAWN: u64 = 15;
const SYS_WAITPID: u64 = 16;
const SYS_KILL: u64 = 17;
const SYS_DISPLAY_INFO: u64 = 18;
const SYS_DISPLAY_CLEAR: u64 = 19;
const SYS_DISPLAY_FILL_RECT: u64 = 20;
const SYS_DISPLAY_DRAW_TEXT: u64 = 21;
const SYS_INPUT_POLL_BYTE: u64 = 22;
const SYS_NET_CONFIGURE: u64 = 23;
const SYS_NET_INFO: u64 = 24;
const SYS_AUDIO_PLAY_TONE: u64 = 25;
const SYS_AUDIO_INFO: u64 = 26;

#[repr(C)]
struct DisplayInfoUser {
    width: u32,
    height: u32,
    pitch: u32,
    bits_per_pixel: u32,
    pixel_format: u32,
}

const SEEK_SET: u64 = 0;
const SEEK_CUR: u64 = 1;
const SEEK_END: u64 = 2;
const WNOHANG: u64 = 1 << 0;

const MAX_IO: usize = 4096;

/// C-ABI entry called by the `syscall` assembly stub in
/// `arch::x86_64::syscall`.
#[no_mangle]
pub extern "C" fn dispatch(a0: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64, num: u64) -> u64 {
    let _ = (a4, a5);
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
        SYS_IPC_PORT_CREATE => ipc_sys::sys_port_create() as u64,
        SYS_IPC_SEND => ipc_sys::sys_send(a0 as i32, a1) as u64,
        SYS_IPC_RECV => ipc_sys::sys_recv(a0 as i32, a1) as u64,
        SYS_IPC_CAP_DROP => ipc_sys::sys_cap_drop(a0 as i32) as u64,
        SYS_IPC_CAP_DUP => ipc_sys::sys_cap_dup(a0 as i32, a1 as u32) as u64,
        SYS_IPC_NAME => ipc_sys::sys_name(a0 as u32, a1, a2, a3 as i32) as u64,
        SYS_SPAWN => sys_spawn(a0 as *const u8, a1 as usize, a2) as u64,
        SYS_WAITPID => sys_waitpid(a0, a1 as *mut i32, a2) as u64,
        SYS_KILL => sys_kill(a0, a1 as i32) as u64,
        SYS_DISPLAY_INFO => sys_display_info(a0 as *mut DisplayInfoUser) as u64,
        SYS_DISPLAY_CLEAR => crate::drivers::framebuffer::clear(a0 as u32) as u64,
        SYS_DISPLAY_FILL_RECT => crate::drivers::framebuffer::fill_rect(
            a0 as u32, a1 as u32, a2 as u32, a3 as u32, a4 as u32,
        ) as u64,
        SYS_DISPLAY_DRAW_TEXT => sys_display_draw_text(
            a0 as u32,
            a1 as u32,
            a2 as *const u8,
            a3 as usize,
            a4 as u32,
        ) as u64,
        SYS_INPUT_POLL_BYTE => crate::drivers::ps2::poll_event_byte().unwrap_or(0) as u64,
        SYS_NET_CONFIGURE => sys_net_configure(a0 as *mut crate::drivers::e1000::NetInfo) as u64,
        SYS_NET_INFO => sys_net_info(a0 as *mut crate::drivers::e1000::NetInfo) as u64,
        SYS_AUDIO_PLAY_TONE => sys_audio_play_tone(
            a0 as u32,
            a1 as u32,
            a2 as *mut crate::drivers::ac97::AudioInfo,
        ) as u64,
        SYS_AUDIO_INFO => sys_audio_info(a0 as *mut crate::drivers::ac97::AudioInfo) as u64,
        _ => -1i64 as u64,
    }
}

fn fs_err_to_code(e: fs::FsError) -> i64 {
    -(e as i64 + 1)
}

fn sys_net_configure(info_ptr: *mut crate::drivers::e1000::NetInfo) -> i64 {
    if info_ptr.is_null() {
        return -1;
    }
    let info = crate::drivers::e1000::configure();
    unsafe {
        core::ptr::write_unaligned(info_ptr, info);
    }
    0
}

fn sys_net_info(info_ptr: *mut crate::drivers::e1000::NetInfo) -> i64 {
    if info_ptr.is_null() {
        return -1;
    }
    let info = crate::drivers::e1000::info();
    unsafe {
        core::ptr::write_unaligned(info_ptr, info);
    }
    0
}

fn sys_audio_play_tone(
    freq_hz: u32,
    duration_ms: u32,
    info_ptr: *mut crate::drivers::ac97::AudioInfo,
) -> i64 {
    if info_ptr.is_null() {
        return -1;
    }
    let info = crate::drivers::ac97::play_tone(freq_hz, duration_ms);
    unsafe {
        core::ptr::write_unaligned(info_ptr, info);
    }
    0
}

fn sys_audio_info(info_ptr: *mut crate::drivers::ac97::AudioInfo) -> i64 {
    if info_ptr.is_null() {
        return -1;
    }
    let info = crate::drivers::ac97::info();
    unsafe {
        core::ptr::write_unaligned(info_ptr, info);
    }
    0
}

fn sys_display_info(info_ptr: *mut DisplayInfoUser) -> i64 {
    if info_ptr.is_null() {
        return -1;
    }
    let info = match crate::drivers::framebuffer::info() {
        Some(info) => info,
        None => return -1,
    };
    unsafe {
        core::ptr::write_unaligned(
            info_ptr,
            DisplayInfoUser {
                width: info.width,
                height: info.height,
                pitch: info.pitch,
                bits_per_pixel: info.bits_per_pixel,
                pixel_format: info.pixel_format,
            },
        );
    }
    0
}

fn sys_display_draw_text(x: u32, y: u32, buf: *const u8, len: usize, color: u32) -> i64 {
    if len > MAX_IO || buf.is_null() {
        return -1;
    }
    let bytes = unsafe { core::slice::from_raw_parts(buf, len) };
    crate::drivers::framebuffer::draw_text(x, y, bytes, color)
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
    if task.fds().lock().close(fd) {
        0
    } else {
        fs_err_to_code(fs::FsError::BadFd)
    }
}

fn sys_spawn(path_ptr: *const u8, path_len: usize, _flags: u64) -> i64 {
    if path_len == 0 || path_len > 256 || path_ptr.is_null() {
        return -1;
    }
    let bytes = unsafe { core::slice::from_raw_parts(path_ptr, path_len) };
    let path = match core::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    let parent = match crate::sched::current_id() {
        Some(pid) => pid,
        None => return -1,
    };
    let child = match crate::user::spawn_from_path(path, alloc::vec::Vec::new(), parent) {
        Ok(t) => t,
        Err(_) => return -1,
    };
    let pid = child.id();
    crate::sched::register(&child);
    crate::sched::enqueue(child);
    pid as i64
}

fn sys_waitpid(pid: u64, status_ptr: *mut i32, flags: u64) -> i64 {
    let parent = match crate::sched::current_id() {
        Some(pid) => pid,
        None => return -1,
    };
    if !crate::sched::has_child(parent, pid) {
        return -1;
    }
    loop {
        if let Some((exited_pid, code)) = crate::sched::find_exited_child(parent, pid) {
            if !status_ptr.is_null() {
                unsafe {
                    core::ptr::write_unaligned(status_ptr, code);
                }
            }
            return exited_pid as i64;
        }
        if flags & WNOHANG != 0 {
            return 0;
        }
        crate::sched::block_on(crate::sched::wait_key(parent));
    }
}

fn sys_kill(pid: u64, code: i32) -> i64 {
    let caller = match crate::sched::current_id() {
        Some(pid) => pid,
        None => return -1,
    };
    if pid == caller {
        crate::sched::current_exit(code);
    }
    let allowed = caller == 2 || crate::sched::is_child(caller, pid);
    if !allowed {
        return -1;
    }
    if crate::sched::kill(pid, code) {
        0
    } else {
        -1
    }
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
