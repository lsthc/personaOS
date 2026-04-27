#![no_std]

use libpersona::{ipc_send, SendMsg, SURFACE_DRAW};
use skipstone::{TextStyle, BODY};

pub struct SurfaceCard<'a> {
    pub x: u32,
    pub y: u32,
    pub accent: u32,
    pub title: &'a [u8],
    pub text_style: TextStyle,
}

impl<'a> SurfaceCard<'a> {
    pub const fn new(title: &'a [u8]) -> Self {
        Self {
            x: 420,
            y: 360,
            accent: 0x7AA2FF,
            title,
            text_style: BODY,
        }
    }

    pub const fn position(mut self, x: u32, y: u32) -> Self {
        self.x = x;
        self.y = y;
        self
    }

    pub const fn accent(mut self, color: u32) -> Self {
        self.accent = color;
        self
    }

    pub const fn text_style(mut self, style: TextStyle) -> Self {
        self.text_style = style;
        self
    }

    pub fn send(&self, surface: i32) -> i64 {
        let msg = SendMsg {
            regs: [
                SURFACE_DRAW,
                self.x as u64,
                self.y as u64,
                self.accent as u64,
                title_id(self.title),
                ((self.text_style.line_height as u64) << 32) | self.text_style.id,
            ],
            caps_ptr: 0,
            ncaps: 0,
            pages_va: 0,
            pages_len: 0,
        };
        ipc_send(surface, &msg)
    }
}

fn title_id(title: &[u8]) -> u64 {
    if title == b"Lily surface" {
        1
    } else {
        0
    }
}
