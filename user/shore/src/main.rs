//! Shore — first personaOS userspace shell.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

use libpersona::{close, exit, getpid, open, read, spawn, waitpid, write, write_dec, write_hex};

#[panic_handler]
fn on_panic(_info: &PanicInfo) -> ! {
    exit(-1)
}

fn eq_cmd(input: &[u8], cmd: &[u8]) -> bool {
    let mut end = 0;
    while end < input.len() && input[end] != b'\n' && input[end] != b'\r' {
        end += 1;
    }
    end == cmd.len() && &input[..end] == cmd
}

fn read_line(buf: &mut [u8]) -> usize {
    let n = read(0, buf);
    if n <= 0 { 0 } else { n as usize }
}

fn cat_hello() {
    let fd = open(b"/hello.txt", 0);
    if fd < 0 {
        write(1, b"shore: open failed\n");
        return;
    }
    let mut buf = [0u8; 64];
    loop {
        let n = read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        write(1, &buf[..n as usize]);
    }
    close(fd);
}

fn run_vfsd() {
    let pid = spawn(b"/sbin/vfsd");
    write(1, b"shore: spawn /sbin/vfsd -> ");
    write_dec(pid);
    write(1, b"\n");
    if pid > 0 {
        let mut status = 0i32;
        let wr = waitpid(pid as u64, &mut status as *mut i32);
        write(1, b"shore: waitpid -> ");
        write_dec(wr);
        write(1, b" status=");
        write_dec(status as i64);
        write(1, b"\n");
    }
}

#[no_mangle]
#[link_section = ".text._start"]
pub extern "C" fn _start() -> ! {
    write(1, b"[shore] starting pid=0x");
    write_hex(1, getpid());
    write(1, b"\n");
    write(1, b"Shore 0.1 - commands: help, hello, run-vfsd, exit\n");

    let mut line = [0u8; 64];
    loop {
        write(1, b"shore> ");
        let n = read_line(&mut line);
        if n == 0 {
            continue;
        }
        if eq_cmd(&line[..n], b"help") {
            write(1, b"help hello run-vfsd exit\n");
        } else if eq_cmd(&line[..n], b"hello") {
            cat_hello();
        } else if eq_cmd(&line[..n], b"run-vfsd") {
            run_vfsd();
        } else if eq_cmd(&line[..n], b"exit") {
            write(1, b"[shore] exiting\n");
            exit(0);
        } else {
            write(1, b"shore: unknown command\n");
        }
    }
}
