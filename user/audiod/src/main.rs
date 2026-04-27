#![no_std]
#![no_main]

use core::panic::PanicInfo;

use libpersona::{
    audio_play_tone, exit, getpid, ipc_name_lookup, ipc_send, write, write_dec, write_hex, AudioInfo,
    SendMsg,
};

const AUDIOD_READY: u64 = 0x6175_646f;

#[panic_handler]
fn on_panic(_info: &PanicInfo) -> ! {
    exit(-1)
}

fn notify_spring(pid: u64, info: &AudioInfo) -> i64 {
    let cap = ipc_name_lookup(b"com.persona.spring");
    write(1, b"[audiod] spring lookup cap=");
    write_dec(cap);
    write(1, b"\n");
    if cap < 0 {
        return cap;
    }
    let msg = SendMsg {
        regs: [
            AUDIOD_READY,
            pid,
            info.played as u64,
            info.sample_rate as u64,
            info.frames as u64,
            0,
        ],
        caps_ptr: 0,
        ncaps: 0,
        pages_va: 0,
        pages_len: 0,
    };
    ipc_send(cap as i32, &msg)
}

#[no_mangle]
#[link_section = ".text._start"]
pub extern "C" fn _start() -> ! {
    let pid = getpid();
    write(1, b"[audiod] starting pid=0x");
    write_hex(1, pid);
    write(1, b"\n");

    let mut info = AudioInfo::empty();
    let rc = audio_play_tone(440, 180, &mut info);
    write(1, b"[audiod] audio_play_tone -> ");
    write_dec(rc);
    write(1, b" present=");
    write_dec(info.present as i64);
    write(1, b" played=");
    write_dec(info.played as i64);
    write(1, b" rate=");
    write_dec(info.sample_rate as i64);
    write(1, b" frames=");
    write_dec(info.frames as i64);
    write(1, b"\n");

    let nr = notify_spring(pid, &info);
    write(1, b"[audiod] notify spring -> ");
    write_dec(nr);
    write(1, b"\n");

    write(1, b"[audiod] exiting\n");
    exit(if rc == 0 && info.played != 0 { 0 } else { 1 });
}
