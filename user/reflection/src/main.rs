//! Reflection — first personaOS compositor smoke.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

use libpersona::{
    display_clear, display_draw_text, display_fill_rect, display_info, exit, getpid, input_poll_byte,
    ipc_cap_dup, ipc_name_lookup, ipc_port_create, ipc_recv, ipc_send, write, write_dec, write_hex,
    DisplayInfo,
    RecvMsg, SendMsg, REFLECTION_READY, RIGHTS_DUP, RIGHTS_SEND, SURFACE_DESKTOP, SURFACE_DRIFT,
    SURFACE_DRAW, SURFACE_SHUTDOWN, SURFACE_SKIM, SURFACE_STONES, SURFACE_TIDE,
};
use skipstone::style_name;

#[panic_handler]
fn on_panic(_info: &PanicInfo) -> ! {
    exit(-1)
}

fn draw_window(x: u32, y: u32, width: u32, height: u32, title: &[u8]) {
    let _ = display_fill_rect(x, y, width, height, 0x182033);
    let _ = display_fill_rect(x + 1, y + 1, width.saturating_sub(2), 24, 0x273552);
    let _ = display_fill_rect(x + 8, y + 8, 8, 8, 0xFF5F57);
    let _ = display_fill_rect(x + 24, y + 8, 8, 8, 0xFFBD2E);
    let _ = display_fill_rect(x + 40, y + 8, 8, 8, 0x28C840);
    let _ = display_draw_text(x + 64, y + 6, title, 0xE6E6FA);
    let _ = display_fill_rect(x + 1, y + 25, width.saturating_sub(2), height.saturating_sub(26), 0x101827);
}

fn draw_desktop(accent: u32, style_id: u64, line_height: u32, dock_apps: u32, status_flags: u32) {
    let style = style_name(style_id);
    let _ = display_clear(0x0B1020);
    let _ = display_fill_rect(0, 0, 2048, 30, 0x101827);
    let _ = display_draw_text(18, 7, b"personaOS", 0xE6E6FA);
    let _ = display_draw_text(128, 7, b"Tide", 0x9FB4FF);
    let _ = display_draw_text(184, 7, b"Skim", 0x9FB4FF);
    let _ = display_draw_text(240, 7, b"Stones", 0x9FB4FF);
    let _ = display_draw_text(312, 7, b"Drift", 0x9FB4FF);
    let _ = display_draw_text(384, 7, b"Depth", 0x9FB4FF);
    if status_flags & 1 != 0 {
        let _ = display_draw_text(1760, 7, b"Wi-Fi", 0x9FB4FF);
    }
    if status_flags & 2 != 0 {
        let _ = display_draw_text(1824, 7, b"Sound", 0x9FB4FF);
    }
    if status_flags & 4 != 0 {
        let _ = display_draw_text(1904, 7, b"Pond", 0x9FB4FF);
    }

    let _ = display_fill_rect(690, 1872, 668, 104, 0x182033);
    let _ = display_fill_rect(718, 1896, 56, 56, accent);
    let _ = display_fill_rect(718, 1960, 32, 4, accent);
    let _ = display_draw_text(708, 1930, b"Tide", 0xE6E6FA);
    let _ = display_fill_rect(820, 1896, 56, 56, 0x4F8CFF);
    let _ = display_fill_rect(820, 1960, 32, 4, 0x4F8CFF);
    let _ = display_draw_text(810, 1930, b"Skim", 0xE6E6FA);
    let _ = display_fill_rect(922, 1896, 56, 56, 0x28C840);
    let _ = display_fill_rect(922, 1960, 32, 4, 0x28C840);
    let _ = display_draw_text(902, 1930, b"Stones", 0xE6E6FA);
    let _ = display_fill_rect(1024, 1896, 56, 56, 0xFFBD2E);
    let _ = display_fill_rect(1024, 1960, 32, 4, 0xFFBD2E);
    let _ = display_draw_text(1014, 1930, b"Drift", 0xE6E6FA);
    let _ = display_fill_rect(1126, 1896, 56, 56, 0xFF5F57);
    let _ = display_fill_rect(1126, 1960, 32, 4, 0xFF5F57);
    let _ = display_draw_text(1116, 1930, b"Depth", 0xE6E6FA);

    draw_window(420, 360, 420, 220, b"Desktop");
    let _ = display_draw_text(444, 416, b"Dock + menu bar smoke", 0xE6E6FA);
    let _ = display_draw_text(444, 440, style, 0x9FB4FF);
    let _ = display_fill_rect(444, 468, line_height.min(32), 8, accent);
    write(1, b"[reflection] drew desktop shell dock_apps=");
    write_dec(dock_apps as i64);
    write(1, b" status_flags=");
    write_dec(status_flags as i64);
    write(1, b" style=");
    write(1, style);
    write(1, b" line_height=");
    write_dec(line_height as i64);
    write(1, b"\n");
}

fn draw_skim(accent: u32, style_id: u64, line_height: u32, entries: u32) {
    let style = style_name(style_id);
    draw_window(900, 360, 520, 320, b"Skim");
    let _ = display_fill_rect(901, 386, 128, 293, 0x121B2B);
    let _ = display_draw_text(924, 416, b"Favorites", 0x9FB4FF);
    let _ = display_draw_text(924, 448, b"Pond", 0xE6E6FA);
    let _ = display_draw_text(924, 472, b"Apps", 0xE6E6FA);
    let _ = display_draw_text(1060, 416, b"/", 0x9FB4FF);
    let _ = display_draw_text(1060, 448, b"hello.txt", 0xE6E6FA);
    let _ = display_draw_text(1060, 472, b"scratch.txt", 0xE6E6FA);
    let _ = display_draw_text(1060, 496, b"bin", 0xE6E6FA);
    let _ = display_draw_text(1060, 520, b"etc", 0xE6E6FA);
    let _ = display_draw_text(1060, 552, style, 0x9FB4FF);
    let _ = display_fill_rect(1060, 584, line_height.min(32), 8, accent);
    let mut i = 0;
    while i < entries.min(5) {
        let _ = display_fill_rect(1048, 444 + i * 24, 4, 12, accent);
        i += 1;
    }
    write(1, b"[reflection] drew Skim file manager entries=");
    write_dec(entries as i64);
    write(1, b" style=");
    write(1, style);
    write(1, b" line_height=");
    write_dec(line_height as i64);
    write(1, b"\n");
}

fn draw_tide(accent: u32, style_id: u64, line_height: u32, results: u32) {
    let style = style_name(style_id);
    let _ = display_fill_rect(560, 180, 928, 360, 0x101827);
    let _ = display_fill_rect(562, 182, 924, 64, 0x182033);
    let _ = display_draw_text(600, 206, b"Tide", 0xE6E6FA);
    let _ = display_draw_text(680, 206, b"skim", 0x9FB4FF);
    let _ = display_fill_rect(600, 260, 848, 2, accent);
    let _ = display_draw_text(620, 292, b"Skim", 0xE6E6FA);
    let _ = display_draw_text(620, 324, b"Stones", 0xE6E6FA);
    let _ = display_draw_text(620, 356, b"Depth", 0xE6E6FA);
    let _ = display_draw_text(620, 388, b"Reflection", 0xE6E6FA);
    let _ = display_draw_text(920, 292, b"File manager", 0x9FB4FF);
    let _ = display_draw_text(920, 324, b"Settings", 0x9FB4FF);
    let _ = display_draw_text(920, 356, b"Terminal", 0x9FB4FF);
    let _ = display_draw_text(920, 388, b"Compositor", 0x9FB4FF);
    let _ = display_draw_text(620, 444, style, 0x9FB4FF);
    let _ = display_fill_rect(620, 476, line_height.min(32), 8, accent);
    let mut i = 0;
    while i < results.min(4) {
        let _ = display_fill_rect(596, 288 + i * 32, 8, 18, accent);
        i += 1;
    }
    write(1, b"[reflection] drew Tide launcher results=");
    write_dec(results as i64);
    write(1, b" style=");
    write(1, style);
    write(1, b" line_height=");
    write_dec(line_height as i64);
    write(1, b"\n");
}

fn draw_drift(accent: u32, style_id: u64, line_height: u32, lines: u32) {
    let style = style_name(style_id);
    draw_window(1080, 760, 560, 340, b"Drift");
    let _ = display_draw_text(1112, 816, b"drift.md", 0x9FB4FF);
    let _ = display_fill_rect(1112, 846, 488, 1, 0x273552);
    let _ = display_draw_text(1112, 880, b"# personaOS", 0xE6E6FA);
    let _ = display_draw_text(1112, 912, b"Calm tools for focused work.", 0xE6E6FA);
    let _ = display_draw_text(1112, 944, b"Local-first. Capability-safe.", 0xE6E6FA);
    let _ = display_draw_text(1112, 992, style, 0x9FB4FF);
    let _ = display_fill_rect(1112, 1024, line_height.min(32), 8, accent);
    let mut i = 0;
    while i < lines.min(4) {
        let _ = display_fill_rect(1096, 876 + i * 32, 4, 18, accent);
        i += 1;
    }
    write(1, b"[reflection] drew Drift editor lines=");
    write_dec(lines as i64);
    write(1, b" style=");
    write(1, style);
    write(1, b" line_height=");
    write_dec(line_height as i64);
    write(1, b"\n");
}

fn draw_stones(accent: u32, style_id: u64, line_height: u32, toggles: u32) {
    let style = style_name(style_id);
    draw_window(500, 720, 520, 300, b"Stones");
    let _ = display_draw_text(532, 776, b"System Settings", 0xE6E6FA);
    let _ = display_draw_text(532, 808, b"Appearance", 0x9FB4FF);
    let _ = display_draw_text(532, 840, b"Privacy", 0x9FB4FF);
    let _ = display_draw_text(532, 872, b"Updates", 0x9FB4FF);
    let _ = display_fill_rect(760, 806, 48, 22, 0x273552);
    let _ = display_fill_rect(786, 810, 14, 14, accent);
    let _ = display_fill_rect(760, 838, 48, 22, 0x273552);
    let _ = display_fill_rect(768, 842, 14, 14, accent);
    let _ = display_fill_rect(760, 870, 48, 22, 0x273552);
    let _ = display_fill_rect(786, 874, 14, 14, accent);
    let _ = display_draw_text(532, 920, style, 0x9FB4FF);
    let _ = display_fill_rect(532, 952, line_height.min(32), 8, accent);
    write(1, b"[reflection] drew Stones settings toggles=");
    write_dec(toggles as i64);
    write(1, b" style=");
    write(1, style);
    write(1, b" line_height=");
    write_dec(line_height as i64);
    write(1, b"\n");
}

fn poll_input_smoke() {
    let mut key = 0;
    for _ in 0..80_000 {
        key = input_poll_byte();
        if key > 0 {
            break;
        }
    }
    if key <= 0 {
        return;
    }
    let _ = display_fill_rect(1628, 4, 112, 22, 0x273552);
    let _ = display_draw_text(1640, 7, b"Key", 0xE6E6FA);
    let _ = display_fill_rect(1684, 10, (key as u32).min(56), 8, 0x7AA2FF);
    write(1, b"[reflection] input key=0x");
    write_hex(1, key as u64);
    write(1, b"\n");
}

fn notify_spring(pid: u64, surface_send: i32) -> i64 {
    let spring = ipc_name_lookup(b"com.persona.spring");
    write(1, b"[reflection] spring lookup cap=");
    write_dec(spring);
    write(1, b"\n");
    if spring < 0 {
        return spring;
    }

    let mut caps = [surface_send];
    let msg = SendMsg {
        regs: [REFLECTION_READY, pid, 0, 0, 0, 0],
        caps_ptr: caps.as_mut_ptr() as u64,
        ncaps: 1,
        pages_va: 0,
        pages_len: 0,
    };
    ipc_send(spring as i32, &msg)
}

fn draw_initial(info: &DisplayInfo) {
    let _ = display_clear(0x0B1020);
    let _ = display_draw_text(32, 28, b"personaOS", 0xE6E6FA);
    let _ = display_draw_text(32, 52, b"M5 desktop seed", 0x9FB4FF);

    let panel_w = if info.width > 360 {
        320
    } else {
        info.width.saturating_sub(48)
    };
    let panel_h = if info.height > 260 {
        180
    } else {
        info.height.saturating_sub(96)
    };
    let panel_x = if info.width > panel_w {
        (info.width - panel_w) / 2
    } else {
        0
    };
    let panel_y = if info.height > panel_h {
        (info.height - panel_h) / 2
    } else {
        0
    };
    draw_window(panel_x, panel_y, panel_w, panel_h, b"Reflection");
    let _ = display_draw_text(panel_x + 24, panel_y + 56, b"waiting for desktop", 0xE6E6FA);
    let _ = display_fill_rect(
        panel_x + 24,
        panel_y + 120,
        panel_w.saturating_sub(48),
        6,
        0x4F8CFF,
    );
}

fn serve(surface_recv: i32) -> i32 {
    loop {
        let mut msg = RecvMsg {
            regs: [0; 6],
            caps_out: 0,
            caps_max: 0,
            ncaps: 0,
            pages_va: 0,
            pages_len: 0,
        };
        let rr = ipc_recv(surface_recv, &mut msg);
        write(1, b"[reflection] surface recv -> ");
        write_dec(rr);
        write(1, b" op=0x");
        write_hex(1, msg.regs[0]);
        write(1, b"\n");
        if rr < 0 {
            return 1;
        }
        poll_input_smoke();
        match msg.regs[0] {
            SURFACE_DRAW => {
                let x = msg.regs[1] as u32;
                let y = msg.regs[2] as u32;
                let color = msg.regs[3] as u32;
                let label = match msg.regs[4] {
                    1 => b"Lily surface" as &[u8],
                    _ => b"client surface" as &[u8],
                };
                let style_id = msg.regs[5] & 0xFFFF_FFFF;
                let line_height = (msg.regs[5] >> 32) as u32;
                let style = style_name(style_id);
                draw_window(x, y, 360, 180, label);
                let _ = display_draw_text(x + 24, y + 56, b"drawn via IPC", 0xE6E6FA);
                let _ = display_draw_text(x + 24, y + 80, style, 0x9FB4FF);
                let _ = display_fill_rect(x + 24, y + 104, line_height.min(32), 8, color);
                let _ = display_fill_rect(x + 24, y + 124, 280, 8, color);
                write(1, b"[reflection] drew client surface style=");
                write(1, style);
                write(1, b" line_height=");
                write_dec(line_height as i64);
                write(1, b"\n");
            }
            SURFACE_DESKTOP => {
                let accent = msg.regs[1] as u32;
                let style_id = msg.regs[2] & 0xFFFF_FFFF;
                let line_height = (msg.regs[2] >> 32) as u32;
                let dock_apps = msg.regs[3] as u32;
                let status_flags = msg.regs[4] as u32;
                draw_desktop(accent, style_id, line_height, dock_apps, status_flags);
            }
            SURFACE_SKIM => {
                let accent = msg.regs[1] as u32;
                let style_id = msg.regs[2] & 0xFFFF_FFFF;
                let line_height = (msg.regs[2] >> 32) as u32;
                let entries = msg.regs[3] as u32;
                draw_skim(accent, style_id, line_height, entries);
            }
            SURFACE_STONES => {
                let accent = msg.regs[1] as u32;
                let style_id = msg.regs[2] & 0xFFFF_FFFF;
                let line_height = (msg.regs[2] >> 32) as u32;
                let toggles = msg.regs[3] as u32;
                draw_stones(accent, style_id, line_height, toggles);
            }
            SURFACE_TIDE => {
                let accent = msg.regs[1] as u32;
                let style_id = msg.regs[2] & 0xFFFF_FFFF;
                let line_height = (msg.regs[2] >> 32) as u32;
                let results = msg.regs[3] as u32;
                draw_tide(accent, style_id, line_height, results);
            }
            SURFACE_DRIFT => {
                let accent = msg.regs[1] as u32;
                let style_id = msg.regs[2] & 0xFFFF_FFFF;
                let line_height = (msg.regs[2] >> 32) as u32;
                let lines = msg.regs[3] as u32;
                draw_drift(accent, style_id, line_height, lines);
            }
            SURFACE_SHUTDOWN => {
                write(1, b"[reflection] shutdown requested\n");
                return 0;
            }
            _ => {
                write(1, b"[reflection] unknown surface op\n");
            }
        }
    }
}

#[no_mangle]
#[link_section = ".text._start"]
pub extern "C" fn _start() -> ! {
    let pid = getpid();
    write(1, b"[reflection] starting pid=0x");
    write_hex(1, pid);
    write(1, b"\n");

    let mut info = DisplayInfo {
        width: 0,
        height: 0,
        pitch: 0,
        bits_per_pixel: 0,
        pixel_format: 0,
    };
    let ir = display_info(&mut info);
    write(1, b"[reflection] display_info -> ");
    write_dec(ir);
    write(1, b" ");
    write_dec(info.width as i64);
    write(1, b"x");
    write_dec(info.height as i64);
    write(1, b"\n");
    if ir < 0 {
        exit(1);
    }

    draw_initial(&info);

    let surface = ipc_port_create();
    write(1, b"[reflection] surface port cap=");
    write_dec(surface as i64);
    write(1, b"\n");
    if surface <= 0 {
        exit(1);
    }

    let surface_send = ipc_cap_dup(surface, RIGHTS_SEND | RIGHTS_DUP);
    write(1, b"[reflection] surface send cap=");
    write_dec(surface_send as i64);
    write(1, b"\n");
    if surface_send <= 0 {
        exit(1);
    }

    let nr = notify_spring(pid, surface_send);
    write(1, b"[reflection] notify spring -> ");
    write_dec(nr);
    write(1, b"\n");
    if nr < 0 {
        exit(1);
    }

    let status = serve(surface);
    write(1, b"[reflection] exiting\n");
    exit(status);
}
