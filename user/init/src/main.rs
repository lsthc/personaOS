//! Spring — personaOS PID 1.
//!
//! Still built as `user/init` and installed at `/init` while the early boot
//! path stabilizes. Runtime responsibility is Spring: own the registrar cap,
//! publish the supervisor port, and eventually spawn/supervise services.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

use libpersona::{
    close, do_yield, exit, getpid, ipc_cap_drop, ipc_cap_dup, ipc_name, ipc_port_create, ipc_recv,
    ipc_send, open, read, spawn, waitpid, write, write_dec, write_hex, FD_CREATE, IPC_NAME_LOOKUP,
    IPC_NAME_PUBLISH, REFLECTION_READY, RIGHTS_DUP, RIGHTS_SEND, RecvMsg, SendMsg,
};

#[panic_handler]
fn on_panic(_info: &PanicInfo) -> ! {
    exit(-1)
}

#[repr(C, align(4096))]
struct PagePayload([u8; 8192]);

static mut PAYLOAD: PagePayload = PagePayload([0; 8192]);

const MAX_MANIFEST: usize = 1400;
const MAX_SERVICES: usize = 11;
const MAX_LABEL: usize = 48;
const MAX_PATH: usize = 64;
const NO_SERVICE: usize = usize::MAX;

const LABEL_VFSD: &[u8] = b"com.persona.vfsd";
const LABEL_NETD: &[u8] = b"com.persona.netd";
const LABEL_AUDIOD: &[u8] = b"com.persona.audiod";
const LABEL_REFLECTION: &[u8] = b"com.persona.reflection";
const LABEL_DESKTOP: &[u8] = b"com.persona.desktop";
const LABEL_TIDE: &[u8] = b"com.persona.tide";
const LABEL_SKIM: &[u8] = b"com.persona.skim";
const LABEL_STONES: &[u8] = b"com.persona.stones";
const LABEL_DRIFT: &[u8] = b"com.persona.drift";
const LABEL_SURFACE_DEMO: &[u8] = b"com.persona.surface-demo";
const LABEL_DEPTH: &[u8] = b"com.persona.depth";

const NETD_READY: u64 = 0x6e65_7464;
const AUDIOD_READY: u64 = 0x6175_646f;

#[derive(Clone, Copy)]
struct ServiceSpec {
    label: [u8; MAX_LABEL],
    label_len: usize,
    path: [u8; MAX_PATH],
    path_len: usize,
    keep_alive: bool,
}

impl ServiceSpec {
    const fn empty() -> Self {
        Self {
            label: [0; MAX_LABEL],
            label_len: 0,
            path: [0; MAX_PATH],
            path_len: 0,
            keep_alive: false,
        }
    }

    fn label(&self) -> &[u8] {
        &self.label[..self.label_len]
    }

    fn path(&self) -> &[u8] {
        &self.path[..self.path_len]
    }

    fn set_label(&mut self, value: &[u8]) {
        self.label_len = copy_field(value, &mut self.label);
    }

    fn set_path(&mut self, value: &[u8]) {
        self.path_len = copy_field(value, &mut self.path);
    }
}

#[derive(Clone, Copy)]
struct ServiceRef<'a> {
    label: &'a [u8],
    path: &'a [u8],
}

struct ServiceManifest {
    services: [ServiceSpec; MAX_SERVICES],
    len: usize,
    loaded: bool,
}

impl ServiceManifest {
    const fn empty() -> Self {
        Self {
            services: [ServiceSpec::empty(); MAX_SERVICES],
            len: 0,
            loaded: false,
        }
    }

    fn service<'a>(&'a self, label: &'static [u8], fallback_path: &'static [u8]) -> Option<ServiceRef<'a>> {
        let mut i = 0;
        while i < self.len {
            let service = &self.services[i];
            if service.label() == label {
                let path = if service.path_len > 0 {
                    service.path()
                } else {
                    fallback_path
                };
                return Some(ServiceRef { label: service.label(), path });
            }
            i += 1;
        }

        if self.loaded {
            None
        } else {
            Some(ServiceRef { label, path: fallback_path })
        }
    }
}

fn copy_field(value: &[u8], dest: &mut [u8]) -> usize {
    let mut i = 0;
    while i < value.len() && i < dest.len() {
        dest[i] = value[i];
        i += 1;
    }
    i
}

fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    while !bytes.is_empty() && is_ascii_space(bytes[0]) {
        bytes = &bytes[1..];
    }
    while !bytes.is_empty() && is_ascii_space(bytes[bytes.len() - 1]) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

fn is_ascii_space(byte: u8) -> bool {
    byte == b' ' || byte == b'\t' || byte == b'\r' || byte == b'\n'
}

fn key_is(line: &[u8], key: &[u8]) -> bool {
    if !line.starts_with(key) {
        return false;
    }
    let rest = trim_ascii(&line[key.len()..]);
    !rest.is_empty() && rest[0] == b'='
}

fn toml_string_value(line: &[u8]) -> Option<&[u8]> {
    let mut i = 0;
    while i < line.len() && line[i] != b'\"' {
        i += 1;
    }
    if i == line.len() {
        return None;
    }
    i += 1;
    let start = i;
    while i < line.len() && line[i] != b'\"' {
        i += 1;
    }
    if i == line.len() {
        return None;
    }
    Some(&line[start..i])
}

fn toml_bool_value(line: &[u8]) -> bool {
    let mut i = 0;
    while i < line.len() && line[i] != b'=' {
        i += 1;
    }
    if i == line.len() {
        return false;
    }
    trim_ascii(&line[i + 1..]).starts_with(b"true")
}

fn parse_manifest(raw: &[u8], manifest: &mut ServiceManifest) {
    let mut pos = 0;
    let mut current = NO_SERVICE;

    while pos < raw.len() {
        let start = pos;
        while pos < raw.len() && raw[pos] != b'\n' {
            pos += 1;
        }

        let line = trim_ascii(&raw[start..pos]);
        if line == b"[[service]]" {
            if manifest.len < MAX_SERVICES {
                current = manifest.len;
                manifest.services[current] = ServiceSpec::empty();
                manifest.len += 1;
            } else {
                current = NO_SERVICE;
            }
        } else if current != NO_SERVICE {
            if key_is(line, b"label") {
                if let Some(value) = toml_string_value(line) {
                    manifest.services[current].set_label(value);
                }
            } else if key_is(line, b"path") {
                if let Some(value) = toml_string_value(line) {
                    manifest.services[current].set_path(value);
                }
            } else if key_is(line, b"keep_alive") {
                manifest.services[current].keep_alive = toml_bool_value(line);
            }
        }

        if pos < raw.len() {
            pos += 1;
        }
    }
}

fn log_missing_service(label: &[u8]) {
    write(1, b"[spring] manifest missing service ");
    write(1, label);
    write(1, b"; skipping\n");
}

fn boot_fs_self_test() {
    write(1, b"[spring] fs self-test: /hello.txt\n");
    let fd = open(b"/hello.txt", 0);
    if fd >= 0 {
        let mut buf = [0u8; 64];
        loop {
            let n = read(fd, &mut buf);
            if n <= 0 {
                break;
            }
            write(1, b"[spring] hello: ");
            write(1, &buf[..n as usize]);
        }
        close(fd);
    } else {
        write(1, b"[spring] open(/hello.txt) failed\n");
    }

    let fd = open(b"/scratch.txt", FD_CREATE);
    if fd >= 0 {
        let payload = b"scratch wrote this\n";
        let n = write(fd, payload);
        write(1, b"[spring] wrote /scratch.txt: ");
        write_dec(n);
        write(1, b"\n");
        close(fd);

        let fd2 = open(b"/scratch.txt", 0);
        if fd2 >= 0 {
            let mut buf = [0u8; 64];
            let r = read(fd2, &mut buf);
            if r > 0 {
                write(1, b"[spring] /scratch.txt: ");
                write(1, &buf[..r as usize]);
            }
            close(fd2);
        }
    } else {
        write(1, b"[spring] open(/scratch.txt, CREATE) failed\n");
    }
}

fn load_manifest(manifest: &mut ServiceManifest) {
    let fd = open(b"/etc/spring.toml", 0);
    if fd < 0 {
        write(1, b"[spring] no /etc/spring.toml yet\n");
        return;
    }

    let mut raw = [0u8; MAX_MANIFEST];
    let mut len = 0;
    loop {
        if len == raw.len() {
            break;
        }
        let n = read(fd, &mut raw[len..]);
        if n <= 0 {
            break;
        }
        len += n as usize;
    }
    close(fd);

    manifest.loaded = true;
    parse_manifest(&raw[..len], manifest);

    write(1, b"[spring] manifest loaded:\n");
    write(1, &raw[..len]);
    write(1, b"[spring] manifest services=");
    write_dec(manifest.len as i64);
    write(1, b"\n");
}

fn supervise_ready_service(supervisor: i32, service: ServiceRef, expected_op: u64) -> bool {
    if supervisor <= 0 {
        write(1, b"[spring] cannot start ");
        write(1, service.label);
        write(1, b": supervisor port unavailable\n");
        return false;
    }

    write(1, b"[spring] spawning ");
    write(1, service.label);
    write(1, b" at ");
    write(1, service.path);
    write(1, b"\n");
    let pid = spawn(service.path);
    write(1, b"[spring] spawn ");
    write(1, service.label);
    write(1, b" -> ");
    write_dec(pid);
    write(1, b"\n");
    if pid <= 0 {
        return false;
    }

    let mut msg = RecvMsg {
        regs: [0; 6],
        caps_out: 0,
        caps_max: 0,
        ncaps: 0,
        pages_va: 0,
        pages_len: 0,
    };
    let rr = ipc_recv(supervisor, &mut msg);
    let ready = rr == 0 && msg.regs[0] == expected_op;
    write(1, b"[spring] supervisor recv -> ");
    write_dec(rr);
    if ready {
        write(1, b"; service ");
        write(1, service.label);
        write(1, b" ready pid=");
        write_dec(msg.regs[1] as i64);
        write(1, b" state=");
        write_dec(msg.regs[2] as i64);
        write(1, b"\n");
    } else {
        write(1, b"; unexpected service message\n");
    }

    let mut status = 0i32;
    let wr = waitpid(pid as u64, &mut status as *mut i32);
    write(1, b"[spring] service ");
    write(1, service.label);
    write(1, b" exited pid=");
    write_dec(wr);
    write(1, b" status=");
    write_dec(status as i64);
    write(1, b"\n");
    ready && status == 0
}

fn supervise_vfsd(supervisor: i32, service: ServiceRef) {
    if supervisor <= 0 {
        write(1, b"[spring] cannot start vfsd: supervisor port unavailable\n");
        return;
    }

    write(1, b"[spring] spawning ");
    write(1, service.label);
    write(1, b" at ");
    write(1, service.path);
    write(1, b"\n");
    let pid = spawn(service.path);
    write(1, b"[spring] spawn vfsd -> ");
    write_dec(pid);
    write(1, b"\n");
    if pid <= 0 {
        return;
    }

    let mut msg = RecvMsg {
        regs: [0; 6],
        caps_out: 0,
        caps_max: 0,
        ncaps: 0,
        pages_va: 0,
        pages_len: 0,
    };
    let rr = ipc_recv(supervisor, &mut msg);
    write(1, b"[spring] supervisor recv -> ");
    write_dec(rr);
    if rr == 0 && msg.regs[0] == 0x7666_7364 {
        write(1, b"; service com.persona.vfsd ready pid=");
        write_dec(msg.regs[1] as i64);
        write(1, b"\n");
    } else {
        write(1, b"; unexpected service message\n");
    }

    let mut status = 0i32;
    let wr = waitpid(pid as u64, &mut status as *mut i32);
    write(1, b"[spring] service com.persona.vfsd exited pid=");
    write_dec(wr);
    write(1, b" status=");
    write_dec(status as i64);
    write(1, b"\n");
}

fn supervise_reflection(
    supervisor: i32,
    reflection: ServiceRef,
    desktop: Option<ServiceRef>,
    tide: Option<ServiceRef>,
    skim: Option<ServiceRef>,
    stones: Option<ServiceRef>,
    drift: Option<ServiceRef>,
    surface_demo: Option<ServiceRef>,
) {
    write(1, b"[spring] launching Reflection compositor at ");
    write(1, reflection.path);
    write(1, b"\n");
    let reflection_pid = spawn(reflection.path);
    write(1, b"[spring] spawn reflection -> ");
    write_dec(reflection_pid);
    write(1, b"\n");
    if reflection_pid <= 0 {
        return;
    }

    let mut caps = [0i32; 1];
    let mut msg = RecvMsg {
        regs: [0; 6],
        caps_out: caps.as_mut_ptr() as u64,
        caps_max: 1,
        ncaps: 0,
        pages_va: 0,
        pages_len: 0,
    };
    let rr = ipc_recv(supervisor, &mut msg);
    write(1, b"[spring] reflection recv -> ");
    write_dec(rr);
    if rr == 0 && msg.regs[0] == REFLECTION_READY && msg.ncaps == 1 {
        write(1, b"; service com.persona.reflection ready pid=");
        write_dec(msg.regs[1] as i64);
        write(1, b" surface_cap=");
        write_dec(caps[0] as i64);
        write(1, b"\n");
    } else {
        write(1, b"; unexpected reflection message\n");
    }

    if caps[0] > 0 {
        let pr = ipc_name(IPC_NAME_PUBLISH, b"com.persona.reflection", caps[0]);
        write(1, b"[spring] published com.persona.reflection -> ");
        write_dec(pr);
        write(1, b"\n");
    }

    if let Some(service) = desktop {
        write(1, b"[spring] launching desktop shell at ");
        write(1, service.path);
        write(1, b"\n");
        let desktop_pid = spawn(service.path);
        write(1, b"[spring] spawn desktop -> ");
        write_dec(desktop_pid);
        write(1, b"\n");

        if desktop_pid > 0 {
            let mut status = 0i32;
            let wr = waitpid(desktop_pid as u64, &mut status as *mut i32);
            write(1, b"[spring] service ");
            write(1, service.label);
            write(1, b" exited pid=");
            write_dec(wr);
            write(1, b" status=");
            write_dec(status as i64);
            write(1, b"\n");
        }
    } else {
        log_missing_service(LABEL_DESKTOP);
    }

    if let Some(service) = tide {
        write(1, b"[spring] launching Tide launcher at ");
        write(1, service.path);
        write(1, b"\n");
        let tide_pid = spawn(service.path);
        write(1, b"[spring] spawn tide -> ");
        write_dec(tide_pid);
        write(1, b"\n");

        if tide_pid > 0 {
            let mut status = 0i32;
            let wr = waitpid(tide_pid as u64, &mut status as *mut i32);
            write(1, b"[spring] service ");
            write(1, service.label);
            write(1, b" exited pid=");
            write_dec(wr);
            write(1, b" status=");
            write_dec(status as i64);
            write(1, b"\n");
        }
    } else {
        log_missing_service(LABEL_TIDE);
    }

    if let Some(service) = skim {
        write(1, b"[spring] launching Skim file manager at ");
        write(1, service.path);
        write(1, b"\n");
        let skim_pid = spawn(service.path);
        write(1, b"[spring] spawn skim -> ");
        write_dec(skim_pid);
        write(1, b"\n");

        if skim_pid > 0 {
            let mut status = 0i32;
            let wr = waitpid(skim_pid as u64, &mut status as *mut i32);
            write(1, b"[spring] service ");
            write(1, service.label);
            write(1, b" exited pid=");
            write_dec(wr);
            write(1, b" status=");
            write_dec(status as i64);
            write(1, b"\n");
        }
    } else {
        log_missing_service(LABEL_SKIM);
    }

    if let Some(service) = stones {
        write(1, b"[spring] launching Stones settings at ");
        write(1, service.path);
        write(1, b"\n");
        let stones_pid = spawn(service.path);
        write(1, b"[spring] spawn stones -> ");
        write_dec(stones_pid);
        write(1, b"\n");

        if stones_pid > 0 {
            let mut status = 0i32;
            let wr = waitpid(stones_pid as u64, &mut status as *mut i32);
            write(1, b"[spring] service ");
            write(1, service.label);
            write(1, b" exited pid=");
            write_dec(wr);
            write(1, b" status=");
            write_dec(status as i64);
            write(1, b"\n");
        }
    } else {
        log_missing_service(LABEL_STONES);
    }

    if let Some(service) = drift {
        write(1, b"[spring] launching Drift editor at ");
        write(1, service.path);
        write(1, b"\n");
        let drift_pid = spawn(service.path);
        write(1, b"[spring] spawn drift -> ");
        write_dec(drift_pid);
        write(1, b"\n");

        if drift_pid > 0 {
            let mut status = 0i32;
            let wr = waitpid(drift_pid as u64, &mut status as *mut i32);
            write(1, b"[spring] service ");
            write(1, service.label);
            write(1, b" exited pid=");
            write_dec(wr);
            write(1, b" status=");
            write_dec(status as i64);
            write(1, b"\n");
        }
    } else {
        log_missing_service(LABEL_DRIFT);
    }

    if let Some(service) = surface_demo {
        write(1, b"[spring] launching surface demo at ");
        write(1, service.path);
        write(1, b"\n");
        let client_pid = spawn(service.path);
        write(1, b"[spring] spawn surface-demo -> ");
        write_dec(client_pid);
        write(1, b"\n");

        if client_pid > 0 {
            let mut status = 0i32;
            let wr = waitpid(client_pid as u64, &mut status as *mut i32);
            write(1, b"[spring] service ");
            write(1, service.label);
            write(1, b" exited pid=");
            write_dec(wr);
            write(1, b" status=");
            write_dec(status as i64);
            write(1, b"\n");
        }
    } else {
        log_missing_service(LABEL_SURFACE_DEMO);
    }

    let mut status = 0i32;
    let wr = waitpid(reflection_pid as u64, &mut status as *mut i32);
    write(1, b"[spring] service com.persona.reflection exited pid=");
    write_dec(wr);
    write(1, b" status=");
    write_dec(status as i64);
    write(1, b"\n");
}

fn supervise_depth(service: ServiceRef) {
    write(1, b"[spring] launching Depth terminal at ");
    write(1, service.path);
    write(1, b"\n");
    let pid = spawn(service.path);
    write(1, b"[spring] spawn depth -> ");
    write_dec(pid);
    write(1, b"\n");
    if pid <= 0 {
        return;
    }

    let mut status = 0i32;
    let wr = waitpid(pid as u64, &mut status as *mut i32);
    write(1, b"[spring] service ");
    write(1, service.label);
    write(1, b" exited pid=");
    write_dec(wr);
    write(1, b" status=");
    write_dec(status as i64);
    write(1, b"\n");
}

fn init_supervisor_ipc() -> i32 {
    write(1, b"[spring] ipc: creating supervisor port\n");
    let p = ipc_port_create();
    write(1, b"[spring] ipc: supervisor cap=");
    write_dec(p as i64);
    write(1, b"\n");
    if p <= 0 {
        return p;
    }

    let name = b"com.persona.spring";
    let pr = ipc_name(IPC_NAME_PUBLISH, name, p);
    write(1, b"[spring] supervisor published as com.persona.spring -> ");
    write_dec(pr);
    write(1, b"\n");

    let lookup_cap = ipc_name(IPC_NAME_LOOKUP, name, 0);
    write(1, b"[spring] supervisor lookup cap=");
    write_dec(lookup_cap);
    write(1, b"\n");

    let send_cap = ipc_cap_dup(p, RIGHTS_SEND | RIGHTS_DUP);
    write(1, b"[spring] ipc: dup SEND cap=");
    write_dec(send_cap as i64);
    write(1, b"\n");

    let send_msg = SendMsg {
        regs: [1, 2, 3, 4, 5, 6],
        caps_ptr: 0,
        ncaps: 0,
        pages_va: 0,
        pages_len: 0,
    };
    let sr = ipc_send(send_cap, &send_msg);
    write(1, b"[spring] ipc: inline send -> ");
    write_dec(sr);
    write(1, b"\n");

    let mut recv_msg = RecvMsg {
        regs: [0; 6],
        caps_out: 0,
        caps_max: 0,
        ncaps: 0,
        pages_va: 0,
        pages_len: 0,
    };
    let rr = ipc_recv(p, &mut recv_msg);
    write(1, b"[spring] ipc: inline recv -> ");
    write_dec(rr);
    if rr == 0 && recv_msg.regs == [1, 2, 3, 4, 5, 6] {
        write(1, b" (regs ok)\n");
    } else {
        write(1, b" (regs MISMATCH)\n");
    }

    let payload_ptr = &raw mut PAYLOAD as *mut u8;
    let greeting = b"hello-from-spring";
    unsafe {
        for (i, &b) in greeting.iter().enumerate() {
            *payload_ptr.add(i) = b;
        }
    }
    let send_msg = SendMsg {
        regs: [0x51A1_0001, 0, 0, 0, 0, 0],
        caps_ptr: 0,
        ncaps: 0,
        pages_va: payload_ptr as u64,
        pages_len: 8192,
    };
    let sr = ipc_send(send_cap, &send_msg);
    write(1, b"[spring] ipc: page-steal send -> ");
    write_dec(sr);
    write(1, b"\n");

    let mut recv_msg = RecvMsg {
        regs: [0; 6],
        caps_out: 0,
        caps_max: 0,
        ncaps: 0,
        pages_va: 0,
        pages_len: 0,
    };
    let rr = ipc_recv(p, &mut recv_msg);
    write(1, b"[spring] ipc: page-steal recv -> ");
    write_dec(rr);
    write(1, b" len=");
    write_dec(recv_msg.pages_len as i64);
    write(1, b" va=0x");
    write_hex(1, recv_msg.pages_va);
    write(1, b"\n");
    if rr == 0 && recv_msg.pages_len >= greeting.len() as u64 {
        let bytes = unsafe { core::slice::from_raw_parts(recv_msg.pages_va as *const u8, greeting.len()) };
        write(1, b"[spring] ipc: payload first bytes: ");
        write(1, bytes);
        write(1, b"\n");
    }

    let mut dummy = RecvMsg {
        regs: [0; 6],
        caps_out: 0,
        caps_max: 0,
        ncaps: 0,
        pages_va: 0,
        pages_len: 0,
    };
    let neg = ipc_recv(send_cap, &mut dummy);
    write(1, b"[spring] ipc: recv-on-send -> ");
    write_dec(neg);
    if neg < 0 {
        write(1, b" (ok, rejected)\n");
    } else {
        write(1, b" (BUG: succeeded)\n");
    }

    let dr = ipc_cap_drop(send_cap);
    write(1, b"[spring] ipc: cap_drop SEND -> ");
    write_dec(dr);
    write(1, b"\n");

    p
}

#[no_mangle]
#[link_section = ".text._start"]
pub extern "C" fn _start() -> ! {
    write(1, b"[spring] starting pid=0x");
    write_hex(1, getpid());
    write(1, b"\n");

    let mut manifest = ServiceManifest::empty();
    load_manifest(&mut manifest);
    boot_fs_self_test();

    for _ in 0..3 {
        write(1, b"[spring] scheduler tick\n");
        do_yield();
    }

    let supervisor = init_supervisor_ipc();

    if let Some(service) = manifest.service(LABEL_VFSD, b"/sbin/vfsd") {
        supervise_vfsd(supervisor, service);
    } else {
        log_missing_service(LABEL_VFSD);
    }

    if let Some(service) = manifest.service(LABEL_NETD, b"/sbin/netd") {
        let _ = supervise_ready_service(supervisor, service, NETD_READY);
    } else {
        log_missing_service(LABEL_NETD);
    }

    if let Some(service) = manifest.service(LABEL_AUDIOD, b"/sbin/audiod") {
        let _ = supervise_ready_service(supervisor, service, AUDIOD_READY);
    } else {
        log_missing_service(LABEL_AUDIOD);
    }

    let reflection = manifest.service(LABEL_REFLECTION, b"/bin/reflection");
    let desktop = manifest.service(LABEL_DESKTOP, b"/bin/desktop");
    let tide = manifest.service(LABEL_TIDE, b"/bin/tide");
    let skim = manifest.service(LABEL_SKIM, b"/bin/skim");
    let stones = manifest.service(LABEL_STONES, b"/bin/stones");
    let drift = manifest.service(LABEL_DRIFT, b"/bin/drift");
    let surface_demo = manifest.service(LABEL_SURFACE_DEMO, b"/bin/surface-demo");
    if let Some(service) = reflection {
        supervise_reflection(supervisor, service, desktop, tide, skim, stones, drift, surface_demo);
    } else {
        log_missing_service(LABEL_REFLECTION);
    }

    if let Some(service) = manifest.service(LABEL_DEPTH, b"/bin/depth") {
        supervise_depth(service);
    } else {
        log_missing_service(LABEL_DEPTH);
    }

    write(1, b"[spring] vfsd broker registration deferred\n");
    write(1, b"[spring] exiting cleanly\n");
    exit(0);
}
