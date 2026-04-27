//! drift — first text editor smoke.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

use libpersona::{exit, getpid, ipc_name_lookup, ipc_send, write, write_dec, write_hex, SendMsg, SURFACE_DRIFT};
use skipstone::BODY;

#[panic_handler]
fn on_panic(_info: &PanicInfo) -> ! {
    exit(-1)
}

fn draw_drift(surface: i32) -> i64 {
    let style = ((BODY.line_height as u64) << 32) | BODY.id;
    let msg = SendMsg {
        regs: [SURFACE_DRIFT, 0xFFBD2E, style, 3, 0, getpid()],
        caps_ptr: 0,
        ncaps: 0,
        pages_va: 0,
        pages_len: 0,
    };
    ipc_send(surface, &msg)
}

#[no_mangle]
#[link_section = ".text._start"]
pub extern "C" fn _start() -> ! {
    write(1, b"[drift] starting pid=0x");
    write_hex(1, getpid());
    write(1, b"\n");

    let surface = ipc_name_lookup(b"com.persona.reflection");
    write(1, b"[drift] reflection lookup cap=");
    write_dec(surface);
    write(1, b"\n");
    if surface <= 0 {
        exit(1);
    }

    let dr = draw_drift(surface as i32);
    write(1, b"[drift] draw drift -> ");
    write_dec(dr);
    write(1, b"\n");
    if dr < 0 {
        exit(1);
    }

    write(1, b"[drift] exiting\n");
    exit(0);
}
