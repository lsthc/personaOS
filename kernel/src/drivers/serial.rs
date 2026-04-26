//! Minimal 16550 UART driver for boot diagnostics.

use core::fmt;

use crate::arch::x86_64::{inb, outb};

pub struct SerialPort {
    base: u16,
}

impl SerialPort {
    /// # Safety
    /// The caller must ensure `base` is the I/O port of a 16550-compatible UART.
    pub const unsafe fn new(base: u16) -> Self {
        Self { base }
    }

    pub fn init(&mut self) {
        unsafe {
            outb(self.base + 1, 0x00); // disable interrupts
            outb(self.base + 3, 0x80); // enable DLAB
            outb(self.base, 0x01); // divisor low (115200 baud)
            outb(self.base + 1, 0x00); // divisor high
            outb(self.base + 3, 0x03); // 8N1
            outb(self.base + 2, 0xC7); // FIFO enable, clear, 14-byte threshold
            outb(self.base + 4, 0x0B); // IRQs enabled, RTS/DSR set
        }
    }

    fn can_write(&self) -> bool {
        (unsafe { inb(self.base + 5) } & 0x20) != 0
    }

    pub fn write_byte(&mut self, b: u8) {
        while !self.can_write() {}
        unsafe { outb(self.base, b) };
    }

    pub fn write_str(&mut self, s: &str) -> fmt::Result {
        for &b in s.as_bytes() {
            if b == b'\n' {
                self.write_byte(b'\r');
            }
            self.write_byte(b);
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub fn write_dec_u32(&mut self, mut v: u32) -> fmt::Result {
        let mut buf = [0u8; 10];
        let mut i = buf.len();
        if v == 0 {
            self.write_byte(b'0');
            return Ok(());
        }
        while v > 0 {
            i -= 1;
            buf[i] = b'0' + (v % 10) as u8;
            v /= 10;
        }
        for &b in &buf[i..] {
            self.write_byte(b);
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub fn write_fmt_args(&mut self, args: fmt::Arguments<'_>) -> fmt::Result {
        fmt::write(self, args)
    }
}

impl fmt::Write for SerialPort {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        SerialPort::write_str(self, s)
    }
}
