//! personaOS init — the first userspace program.
//!
//! Tiny, no-std, no-alloc. Talks to the kernel via raw `syscall` only; no
//! libc. This is the ELF the kernel loads into ring 3 on boot.

#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;

#[panic_handler]
fn on_panic(_info: &PanicInfo) -> ! {
    // If anything panics in init, just exit(-1).
    unsafe { syscall1(SYS_EXIT, u64::MAX) };
    loop {}
}

const SYS_WRITE: u64 = 0;
const SYS_EXIT: u64 = 1;
const SYS_YIELD: u64 = 2;
const SYS_GETPID: u64 = 3;
const SYS_OPEN: u64 = 4;
const SYS_READ: u64 = 5;
const SYS_CLOSE: u64 = 6;

const FD_CREATE: u32 = 1 << 2;

#[inline]
unsafe fn syscall0(n: u64) -> u64 {
    let mut ret: u64;
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
unsafe fn syscall1(n: u64, a0: u64) -> u64 {
    let mut ret: u64;
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
unsafe fn syscall3(n: u64, a0: u64, a1: u64, a2: u64) -> u64 {
    let mut ret: u64;
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

fn write(fd: i32, s: &[u8]) -> i64 {
    unsafe { syscall3(SYS_WRITE, fd as u64, s.as_ptr() as u64, s.len() as u64) as i64 }
}

fn do_yield() {
    unsafe { syscall0(SYS_YIELD) };
}

fn getpid() -> u64 {
    unsafe { syscall0(SYS_GETPID) }
}

fn open(path: &[u8], flags: u32) -> i32 {
    unsafe { syscall3(SYS_OPEN, path.as_ptr() as u64, path.len() as u64, flags as u64) as i32 }
}

fn read(fd: i32, buf: &mut [u8]) -> i64 {
    unsafe { syscall3(SYS_READ, fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64) as i64 }
}

fn close(fd: i32) {
    unsafe { syscall1(SYS_CLOSE, fd as u64) };
}

fn exit(code: i32) -> ! {
    unsafe { syscall1(SYS_EXIT, code as u64) };
    loop {}
}

fn write_hex(fd: i32, mut v: u64) {
    let mut buf = [0u8; 16];
    for i in 0..16 {
        let nib = (v & 0xF) as u8;
        buf[15 - i] = if nib < 10 { b'0' + nib } else { b'a' + nib - 10 };
        v >>= 4;
    }
    write(fd, &buf);
}

#[no_mangle]
#[link_section = ".text._start"]
pub extern "C" fn _start() -> ! {
    write(1, b"[init] hello from userspace\n");
    let pid = getpid();
    write(1, b"[init] pid=0x");
    write_hex(1, pid);
    write(1, b"\n");

    // Read /hello.txt from the VFS and echo it.
    let fd = open(b"/hello.txt", 0);
    if fd >= 0 {
        let mut buf = [0u8; 64];
        loop {
            let n = read(fd, &mut buf);
            if n <= 0 {
                break;
            }
            write(1, b"[init] read: ");
            write(1, &buf[..n as usize]);
        }
        close(fd);
    } else {
        write(1, b"[init] open(/hello.txt) failed\n");
    }

    for _ in 0..3 {
        write(1, b"[init] tick\n");
        do_yield();
    }

    // Exercise PondFS write: create /scratch.txt, write into it, re-open,
    // read back, echo.
    let fd = open(b"/scratch.txt", FD_CREATE);
    if fd >= 0 {
        let payload = b"scratch wrote this\n";
        let n = unsafe {
            syscall3(SYS_WRITE, fd as u64, payload.as_ptr() as u64, payload.len() as u64) as i64
        };
        write(1, b"[init] wrote /scratch.txt: ");
        write_dec(n);
        write(1, b"\n");
        close(fd);

        let fd2 = open(b"/scratch.txt", 0);
        if fd2 >= 0 {
            let mut buf = [0u8; 64];
            let r = read(fd2, &mut buf);
            if r > 0 {
                write(1, b"[init] /scratch.txt: ");
                write(1, &buf[..r as usize]);
            }
            close(fd2);
        }
    } else {
        write(1, b"[init] open(/scratch.txt, CREATE) failed\n");
    }

    write(1, b"[init] bye\n");
    exit(0);
}

fn write_dec(mut v: i64) {
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
