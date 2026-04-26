//! Very small framebuffer renderer: solid fills and 8x16 bitmap text.
//!
//! The bootloader hands us a linear framebuffer through the HHDM. We assume
//! 32-bit pixels (the format only affects the channel order).

use persona_shared::{Framebuffer, PixelFormat};

mod font;

pub struct FramebufferConsole {
    fb: Framebuffer,
}

impl FramebufferConsole {
    /// # Safety
    ///
    /// `fb.base` must point to a valid linear framebuffer of at least
    /// `fb.pitch * fb.height` bytes that is exclusively owned for the
    /// lifetime of this console.
    pub unsafe fn new(fb: Framebuffer) -> Self {
        Self { fb }
    }

    pub fn clear(&mut self, color: u32) {
        let px = self.encode(color);
        for y in 0..self.fb.height {
            for x in 0..self.fb.width {
                unsafe { self.put_raw(x, y, px) };
            }
        }
    }

    pub fn draw_string(&mut self, x: u32, y: u32, text: &str, color: u32) {
        let mut cx = x;
        for ch in text.chars() {
            self.draw_char(cx, y, ch, color);
            cx += 8;
            if cx + 8 > self.fb.width {
                break;
            }
        }
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
            PixelFormat::Bgrx8888 => (r << 16) | (g << 8) | b, // memory order B,G,R,x
            PixelFormat::Rgbx8888 => (b << 16) | (g << 8) | r, // memory order R,G,B,x
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
