//! surface-demo — first pond-surface IPC client.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

use libpersona::{exit, getpid, ipc_name_lookup, ipc_send, write, write_dec, write_hex, SendMsg, SURFACE_SHUTDOWN};
use lily::SurfaceCard;
use skipstone::CAPTION;

#[panic_handler]
fn on_panic(_info: &PanicInfo) -> ! {
    exit(-1)
}

fn send_draw(surface: i32) -> i64 {
    SurfaceCard::new(b"Lily surface")
        .position(420, 360)
        .accent(0x7AA2FF)
        .text_style(CAPTION)
        .send(surface)
}

fn send_shutdown(surface: i32) -> i64 {
    let msg = SendMsg {
        regs: [SURFACE_SHUTDOWN, 0, 0, 0, 0, 0],
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
    write(1, b"[surface-demo] starting pid=0x");
    write_hex(1, getpid());
    write(1, b"\n");

    let surface = ipc_name_lookup(b"com.persona.reflection");
    write(1, b"[surface-demo] reflection lookup cap=");
    write_dec(surface);
    write(1, b"\n");
    if surface <= 0 {
        exit(1);
    }

    let dr = send_draw(surface as i32);
    write(1, b"[surface-demo] draw -> ");
    write_dec(dr);
    write(1, b"\n");
    if dr < 0 {
        exit(1);
    }

    let sr = send_shutdown(surface as i32);
    write(1, b"[surface-demo] shutdown -> ");
    write_dec(sr);
    write(1, b"\n");
    if sr < 0 {
        exit(1);
    }

    write(1, b"[surface-demo] exiting\n");
    exit(0);
}
