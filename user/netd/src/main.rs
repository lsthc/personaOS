#![no_std]
#![no_main]

use core::panic::PanicInfo;

use libpersona::{
    exit, getpid, ipc_name_lookup, ipc_send, net_configure, write, write_dec, write_hex, NetInfo,
    SendMsg,
};

const NETD_READY: u64 = 0x6e65_7464;

#[panic_handler]
fn on_panic(_info: &PanicInfo) -> ! {
    exit(-1)
}

fn write_ipv4(addr: [u8; 4]) {
    write_dec(addr[0] as i64);
    write(1, b".");
    write_dec(addr[1] as i64);
    write(1, b".");
    write_dec(addr[2] as i64);
    write(1, b".");
    write_dec(addr[3] as i64);
}

fn notify_spring(pid: u64, info: &NetInfo) -> i64 {
    let cap = ipc_name_lookup(b"com.persona.spring");
    write(1, b"[netd] spring lookup cap=");
    write_dec(cap);
    write(1, b"\n");
    if cap < 0 {
        return cap;
    }
    let ip = ((info.ipv4[0] as u64) << 24)
        | ((info.ipv4[1] as u64) << 16)
        | ((info.ipv4[2] as u64) << 8)
        | info.ipv4[3] as u64;
    let msg = SendMsg {
        regs: [NETD_READY, pid, info.configured as u64, ip, info.tx_packets as u64, info.rx_packets as u64],
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
    write(1, b"[netd] starting pid=0x");
    write_hex(1, pid);
    write(1, b"\n");

    let mut info = NetInfo::empty();
    let rc = net_configure(&mut info);
    write(1, b"[netd] net_configure -> ");
    write_dec(rc);
    write(1, b" present=");
    write_dec(info.present as i64);
    write(1, b" link=");
    write_dec(info.link_up as i64);
    write(1, b" configured=");
    write_dec(info.configured as i64);
    write(1, b" ip=");
    write_ipv4(info.ipv4);
    write(1, b" tx=");
    write_dec(info.tx_packets as i64);
    write(1, b" rx=");
    write_dec(info.rx_packets as i64);
    write(1, b"\n");

    let nr = notify_spring(pid, &info);
    write(1, b"[netd] notify spring -> ");
    write_dec(nr);
    write(1, b"\n");

    write(1, b"[netd] exiting\n");
    exit(if rc == 0 && info.configured != 0 { 0 } else { 1 });
}
