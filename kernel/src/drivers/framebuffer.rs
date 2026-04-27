//! Very small framebuffer renderer: solid fills and 8x16 bitmap text.
//!
//! The bootloader hands us a linear framebuffer through the HHDM. We assume
//! 32-bit pixels (the format only affects the channel order).

use persona_shared::{Framebuffer, PixelFormat};
use spin::Mutex;

mod font;

static DISPLAY: Mutex<Option<FramebufferConsole>> = Mutex::new(None);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DisplayInfo {
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bits_per_pixel: u32,
    pub pixel_format: u32,
}

pub struct FramebufferConsole {
    fb: Framebuffer,
}

unsafe impl Send for FramebufferConsole {}

impl FramebufferConsole {
    /// # Safety
    ///
    /// `fb.base` must point to a valid linear framebuffer of at least
    /// `fb.pitch * fb.height` bytes that is exclusively owned for the
    /// lifetime of this console.
    pub unsafe fn new(fb: Framebuffer) -> Self {
        Self { fb }
    }

    pub fn info(&self) -> DisplayInfo {
        DisplayInfo {
            width: self.fb.width,
            height: self.fb.height,
            pitch: self.fb.pitch,
            bits_per_pixel: self.fb.bits_per_pixel,
            pixel_format: self.fb.pixel_format as u32,
        }
    }

    pub fn clear(&mut self, color: u32) {
        self.fill_rect(0, 0, self.fb.width, self.fb.height, color);
    }

    pub fn fill_rect(&mut self, x: u32, y: u32, width: u32, height: u32, color: u32) {
        if x >= self.fb.width || y >= self.fb.height || width == 0 || height == 0 {
            return;
        }
        let x_end = x.saturating_add(width).min(self.fb.width);
        let y_end = y.saturating_add(height).min(self.fb.height);
        let px = self.encode(color);
        for dy in y..y_end {
            for dx in x..x_end {
                unsafe { self.put_raw(dx, dy, px) };
            }
        }
    }

    pub fn draw_bytes(&mut self, x: u32, mut y: u32, text: &[u8], color: u32) -> usize {
        let mut cx = x;
        let mut drawn = 0;
        for &b in text {
            if b == b'\n' {
                cx = x;
                y = y.saturating_add(16);
                if y + 16 > self.fb.height {
                    break;
                }
                continue;
            }
            if cx + 8 > self.fb.width {
                break;
            }
            let ch = if (0x20..=0x7E).contains(&b) {
                b as char
            } else {
                ' '
            };
            self.draw_char(cx, y, ch, color);
            drawn += 1;
            cx += 8;
        }
        drawn
    }

    fn draw_char(&mut self, x: u32, y: u32, ch: char, color: u32) {
        let glyph = font::glyph(ch);
        let px = self.encode(color);
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..8 {
                if (bits >> (7 - col)) & 1 == 1 {
                    let dx = x + col as u32;
                    let dy = y + row as u32;
                    if dx < self.fb.width && dy < self.fb.height {
                        unsafe { self.put_raw(dx, dy, px) };
                    }
                }
            }
        }
    }

    fn encode(&self, rgb: u32) -> u32 {
        let r = (rgb >> 16) & 0xFF;
        let g = (rgb >> 8) & 0xFF;
        let b = rgb & 0xFF;
        match self.fb.pixel_format {
            PixelFormat::Bgrx8888 => (r << 16) | (g << 8) | b,
            PixelFormat::Rgbx8888 => (b << 16) | (g << 8) | r,
            PixelFormat::Unknown => rgb,
        }
    }

    /// # Safety
    ///
    /// `x < width` and `y < height` must hold.
    unsafe fn put_raw(&mut self, x: u32, y: u32, pixel: u32) {
        let offset = y as usize * self.fb.pitch as usize + x as usize * 4;
        unsafe {
            let ptr = self.fb.base.add(offset) as *mut u32;
            ptr.write_volatile(pixel);
        }
    }
}

pub fn init(fb: Framebuffer) {
    *DISPLAY.lock() = Some(unsafe { FramebufferConsole::new(fb) });
}

pub fn info() -> Option<DisplayInfo> {
    DISPLAY.lock().as_ref().map(FramebufferConsole::info)
}

pub fn clear(color: u32) -> i64 {
    match DISPLAY.lock().as_mut() {
        Some(display) => {
            display.clear(color);
            0
        }
        None => -1,
    }
}

pub fn fill_rect(x: u32, y: u32, width: u32, height: u32, color: u32) -> i64 {
    match DISPLAY.lock().as_mut() {
        Some(display) => {
            display.fill_rect(x, y, width, height, color);
            0
        }
        None => -1,
    }
}

pub fn draw_text(x: u32, y: u32, bytes: &[u8], color: u32) -> i64 {
    match DISPLAY.lock().as_mut() {
        Some(display) => display.draw_bytes(x, y, bytes, color) as i64,
        None => -1,
    }
}
