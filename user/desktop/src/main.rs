//! desktop — first M5 desktop shell smoke.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

use libpersona::{
    audio_info, exit, getpid, ipc_name_lookup, ipc_send, net_info, write, write_dec, write_hex,
    AudioInfo, NetInfo, SendMsg, SURFACE_DESKTOP,
};
use skipstone::BODY;

#[panic_handler]
fn on_panic(_info: &PanicInfo) -> ! {
    exit(-1)
}

fn draw_desktop(surface: i32) -> i64 {
    let style = ((BODY.line_height as u64) << 32) | BODY.id;
    let dock_apps = 5;
    let mut net = NetInfo::empty();
    let mut audio = AudioInfo::empty();
    let _ = net_info(&mut net);
    let _ = audio_info(&mut audio);
    let mut status_flags = 0b100;
    if net.configured != 0 {
        status_flags |= 0b001;
    }
    if audio.played != 0 {
        status_flags |= 0b010;
    }
    let msg = SendMsg {
        regs: [SURFACE_DESKTOP, 0x7AA2FF, style, dock_apps, status_flags, getpid()],
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
    write(1, b"[desktop] starting pid=0x");
    write_hex(1, getpid());
    write(1, b"\n");

    let surface = ipc_name_lookup(b"com.persona.reflection");
    write(1, b"[desktop] reflection lookup cap=");
    write_dec(surface);
    write(1, b"\n");
    if surface <= 0 {
        exit(1);
    }

    let dr = draw_desktop(surface as i32);
    write(1, b"[desktop] draw desktop -> ");
    write_dec(dr);
    write(1, b"\n");
    if dr < 0 {
        exit(1);
    }

    write(1, b"[desktop] exiting\n");
    exit(0);
}
