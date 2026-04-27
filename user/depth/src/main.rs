//! Depth — first personaOS terminal host.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

use libpersona::{exit, getpid, spawn, waitpid, write, write_dec, write_hex};

#[panic_handler]
fn on_panic(_info: &PanicInfo) -> ! {
    exit(-1)
}

#[no_mangle]
#[link_section = ".text._start"]
pub extern "C" fn _start() -> ! {
    let pid = getpid();
    write(1, b"[depth] starting pid=0x");
    write_hex(1, pid);
    write(1, b"\n");

    write(1, b"[depth] launching Shore at /bin/shore\n");
    let child = spawn(b"/bin/shore");
    write(1, b"[depth] spawn shore -> ");
    write_dec(child);
    write(1, b"\n");
    if child <= 0 {
        exit(1);
    }

    let mut status = 0i32;
    let wr = waitpid(child as u64, &mut status as *mut i32);
    write(1, b"[depth] shore exited pid=");
    write_dec(wr);
    write(1, b" status=");
    write_dec(status as i64);
    write(1, b"\n");

    write(1, b"[depth] exiting\n");
    exit(status);
}
