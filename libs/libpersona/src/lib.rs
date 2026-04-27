#![no_std]

use core::arch::asm;

pub const SYS_WRITE: u64 = 0;
pub const SYS_EXIT: u64 = 1;
pub const SYS_YIELD: u64 = 2;
pub const SYS_GETPID: u64 = 3;
pub const SYS_OPEN: u64 = 4;
pub const SYS_READ: u64 = 5;
pub const SYS_CLOSE: u64 = 6;
pub const SYS_LSEEK: u64 = 7;
pub const SYS_FSTAT: u64 = 8;
pub const SYS_IPC_PORT_CREATE: u64 = 9;
pub const SYS_IPC_SEND: u64 = 10;
pub const SYS_IPC_RECV: u64 = 11;
pub const SYS_IPC_CAP_DROP: u64 = 12;
pub const SYS_IPC_CAP_DUP: u64 = 13;
pub const SYS_IPC_NAME: u64 = 14;
pub const SYS_SPAWN: u64 = 15;
pub const SYS_WAITPID: u64 = 16;
pub const SYS_KILL: u64 = 17;
pub const SYS_DISPLAY_INFO: u64 = 18;
pub const SYS_DISPLAY_CLEAR: u64 = 19;
pub const SYS_DISPLAY_FILL_RECT: u64 = 20;
pub const SYS_DISPLAY_DRAW_TEXT: u64 = 21;
pub const SYS_INPUT_POLL_BYTE: u64 = 22;
pub const SYS_NET_CONFIGURE: u64 = 23;
pub const SYS_NET_INFO: u64 = 24;
pub const SYS_AUDIO_PLAY_TONE: u64 = 25;
pub const SYS_AUDIO_INFO: u64 = 26;

pub const FD_READ: u32 = 1 << 0;
pub const FD_WRITE: u32 = 1 << 1;
pub const FD_CREATE: u32 = 1 << 2;

pub const RIGHTS_SEND: u32 = 1 << 0;
pub const RIGHTS_RECV: u32 = 1 << 1;
pub const RIGHTS_DUP: u32 = 1 << 2;

pub const IPC_NAME_PUBLISH: u32 = 0;
pub const IPC_NAME_LOOKUP: u32 = 1;

pub const REFLECTION_READY: u64 = 0x7265_666c_0000_0001;
pub const SURFACE_DRAW: u64 = 0x7375_7266_0000_0001;
pub const SURFACE_SHUTDOWN: u64 = 0x7375_7266_0000_0002;
pub const SURFACE_DESKTOP: u64 = 0x7375_7266_0000_0003;
pub const SURFACE_SKIM: u64 = 0x7375_7266_0000_0004;
pub const SURFACE_STONES: u64 = 0x7375_7266_0000_0005;
pub const SURFACE_TIDE: u64 = 0x7375_7266_0000_0006;
pub const SURFACE_DRIFT: u64 = 0x7375_7266_0000_0007;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DisplayInfo {
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bits_per_pixel: u32,
    pub pixel_format: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NetInfo {
    pub present: u32,
    pub link_up: u32,
    pub configured: u32,
    pub tx_packets: u32,
    pub rx_packets: u32,
    pub mac: [u8; 6],
    pub _pad: [u8; 2],
    pub ipv4: [u8; 4],
    pub router: [u8; 4],
    pub dns: [u8; 4],
}

impl NetInfo {
    pub const fn empty() -> Self {
        Self {
            present: 0,
            link_up: 0,
            configured: 0,
            tx_packets: 0,
            rx_packets: 0,
            mac: [0; 6],
            _pad: [0; 2],
            ipv4: [0; 4],
            router: [0; 4],
            dns: [0; 4],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AudioInfo {
    pub present: u32,
    pub played: u32,
    pub sample_rate: u32,
    pub frames: u32,
}

impl AudioInfo {
    pub const fn empty() -> Self {
        Self {
            present: 0,
            played: 0,
            sample_rate: 0,
            frames: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SendMsg {
    pub regs: [u64; 6],
    pub caps_ptr: u64,
    pub ncaps: u64,
    pub pages_va: u64,
    pub pages_len: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct RecvMsg {
    pub regs: [u64; 6],
    pub caps_out: u64,
    pub caps_max: u64,
    pub ncaps: u64,
    pub pages_va: u64,
    pub pages_len: u64,
}

#[inline]
pub unsafe fn syscall0(n: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") n => ret,
            lateout("rcx") _, lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

#[inline]
pub unsafe fn syscall1(n: u64, a0: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") n => ret,
            in("rdi") a0,
            lateout("rcx") _, lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

#[inline]
pub unsafe fn syscall2(n: u64, a0: u64, a1: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") n => ret,
            in("rdi") a0,
            in("rsi") a1,
            lateout("rcx") _, lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

#[inline]
pub unsafe fn syscall3(n: u64, a0: u64, a1: u64, a2: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") n => ret,
            in("rdi") a0,
            in("rsi") a1,
            in("rdx") a2,
            lateout("rcx") _, lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

#[inline]
pub unsafe fn syscall4(n: u64, a0: u64, a1: u64, a2: u64, a3: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") n => ret,
            in("rdi") a0,
            in("rsi") a1,
            in("rdx") a2,
            in("r10") a3,
            lateout("rcx") _, lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

#[inline]
pub unsafe fn syscall5(n: u64, a0: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") n => ret,
            in("rdi") a0,
            in("rsi") a1,
            in("rdx") a2,
            in("r10") a3,
            in("r8") a4,
            lateout("rcx") _, lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

pub fn write(fd: i32, s: &[u8]) -> i64 {
    unsafe { syscall3(SYS_WRITE, fd as u64, s.as_ptr() as u64, s.len() as u64) as i64 }
}

pub fn do_yield() {
    unsafe { syscall0(SYS_YIELD) };
}

pub fn getpid() -> u64 {
    unsafe { syscall0(SYS_GETPID) }
}

pub fn open(path: &[u8], flags: u32) -> i32 {
    unsafe { syscall3(SYS_OPEN, path.as_ptr() as u64, path.len() as u64, flags as u64) as i32 }
}

pub fn read(fd: i32, buf: &mut [u8]) -> i64 {
    unsafe { syscall3(SYS_READ, fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64) as i64 }
}

pub fn close(fd: i32) {
    unsafe { syscall1(SYS_CLOSE, fd as u64) };
}

pub fn exit(code: i32) -> ! {
    unsafe { syscall1(SYS_EXIT, code as u64) };
    loop {}
}

pub fn spawn(path: &[u8]) -> i64 {
    unsafe { syscall3(SYS_SPAWN, path.as_ptr() as u64, path.len() as u64, 0) as i64 }
}

pub fn waitpid(pid: u64, status: *mut i32) -> i64 {
    unsafe { syscall3(SYS_WAITPID, pid, status as u64, 0) as i64 }
}

pub fn kill(pid: u64, code: i32) -> i64 {
    unsafe { syscall2(SYS_KILL, pid, code as u64) as i64 }
}

pub fn display_info(info: &mut DisplayInfo) -> i64 {
    unsafe { syscall1(SYS_DISPLAY_INFO, info as *mut DisplayInfo as u64) as i64 }
}

pub fn display_clear(color: u32) -> i64 {
    unsafe { syscall1(SYS_DISPLAY_CLEAR, color as u64) as i64 }
}

pub fn display_fill_rect(x: u32, y: u32, width: u32, height: u32, color: u32) -> i64 {
    unsafe {
        syscall5(
            SYS_DISPLAY_FILL_RECT,
            x as u64,
            y as u64,
            width as u64,
            height as u64,
            color as u64,
        ) as i64
    }
}

pub fn display_draw_text(x: u32, y: u32, text: &[u8], color: u32) -> i64 {
    unsafe {
        syscall5(
            SYS_DISPLAY_DRAW_TEXT,
            x as u64,
            y as u64,
            text.as_ptr() as u64,
            text.len() as u64,
            color as u64,
        ) as i64
    }
}

pub fn input_poll_byte() -> i64 {
    unsafe { syscall0(SYS_INPUT_POLL_BYTE) as i64 }
}

pub fn net_configure(info: &mut NetInfo) -> i64 {
    unsafe { syscall1(SYS_NET_CONFIGURE, info as *mut NetInfo as u64) as i64 }
}

pub fn net_info(info: &mut NetInfo) -> i64 {
    unsafe { syscall1(SYS_NET_INFO, info as *mut NetInfo as u64) as i64 }
}

pub fn audio_play_tone(freq_hz: u32, duration_ms: u32, info: &mut AudioInfo) -> i64 {
    unsafe {
        syscall3(
            SYS_AUDIO_PLAY_TONE,
            freq_hz as u64,
            duration_ms as u64,
            info as *mut AudioInfo as u64,
        ) as i64
    }
}

pub fn audio_info(info: &mut AudioInfo) -> i64 {
    unsafe { syscall1(SYS_AUDIO_INFO, info as *mut AudioInfo as u64) as i64 }
}

pub fn ipc_port_create() -> i32 {
    unsafe { syscall0(SYS_IPC_PORT_CREATE) as i32 }
}

pub fn ipc_send(cap: i32, msg: &SendMsg) -> i64 {
    unsafe { syscall2(SYS_IPC_SEND, cap as u64, msg as *const SendMsg as u64) as i64 }
}

pub fn ipc_recv(cap: i32, msg: &mut RecvMsg) -> i64 {
    unsafe { syscall2(SYS_IPC_RECV, cap as u64, msg as *mut RecvMsg as u64) as i64 }
}

pub fn ipc_cap_drop(cap: i32) -> i64 {
    unsafe { syscall1(SYS_IPC_CAP_DROP, cap as u64) as i64 }
}

pub fn ipc_cap_dup(cap: i32, mask: u32) -> i32 {
    unsafe { syscall2(SYS_IPC_CAP_DUP, cap as u64, mask as u64) as i32 }
}

pub fn ipc_name(op: u32, name: &[u8], cap: i32) -> i64 {
    unsafe {
        syscall4(
            SYS_IPC_NAME,
            op as u64,
            name.as_ptr() as u64,
            name.len() as u64,
            cap as u64,
        ) as i64
    }
}

pub fn ipc_name_lookup(name: &[u8]) -> i64 {
    ipc_name(IPC_NAME_LOOKUP, name, 0)
}

pub fn write_hex(fd: i32, mut v: u64) {
    let mut buf = [0u8; 16];
    for i in 0..16 {
        let nib = (v & 0xF) as u8;
        buf[15 - i] = if nib < 10 { b'0' + nib } else { b'a' + nib - 10 };
        v >>= 4;
    }
    write(fd, &buf);
}

pub fn write_dec(mut v: i64) {
    if v < 0 {
        write(1, b"-");
        v = -v;
    }
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    if v == 0 {
        write(1, b"0");
        return;
    }
    while v > 0 {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    write(1, &buf[i..]);
}
