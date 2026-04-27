//! vfsd — first spawned personaOS userspace service.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

use libpersona::{
    close, exit, getpid, ipc_name_lookup, ipc_send, open, read, write, write_dec, write_hex,
    SendMsg,
};

#[panic_handler]
fn on_panic(_info: &PanicInfo) -> ! {
    exit(-1)
}

fn read_hello() {
    let fd = open(b"/hello.txt", 0);
    if fd < 0 {
        write(1, b"[vfsd] open /hello.txt failed\n");
        return;
    }
    let mut buf = [0u8; 64];
    let n = read(fd, &mut buf);
    if n > 0 {
        write(1, b"[vfsd] read /hello.txt: ");
        write(1, &buf[..n as usize]);
    } else {
        write(1, b"[vfsd] read /hello.txt returned ");
        write_dec(n);
        write(1, b"\n");
    }
    close(fd);
}

fn notify_spring(pid: u64) -> i64 {
    let cap = ipc_name_lookup(b"com.persona.spring");
    write(1, b"[vfsd] spring lookup cap=");
    write_dec(cap);
    write(1, b"\n");
    if cap < 0 {
        return cap;
    }
    let msg = SendMsg {
        regs: [0x7666_7364, pid, 0, 0, 0, 0],
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
    write(1, b"[vfsd] starting pid=0x");
    write_hex(1, pid);
    write(1, b"\n");

    read_hello();

    let nr = notify_spring(pid);
    write(1, b"[vfsd] notify spring -> ");
    write_dec(nr);
    write(1, b"\n");

    write(1, b"[vfsd] exiting\n");
    exit(0);
}
