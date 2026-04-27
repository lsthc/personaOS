//! PS/2 controller keyboard support.

use core::hint::spin_loop;

use spin::Mutex;

use crate::arch::x86_64::{inb, outb};

const DATA_PORT: u16 = 0x60;
const STATUS_PORT: u16 = 0x64;

const STATUS_OUTPUT_FULL: u8 = 1 << 0;
const STATUS_INPUT_FULL: u8 = 1 << 1;
const STATUS_AUX_DATA: u8 = 1 << 5;

const INPUT_QUEUE: usize = 32;

static INPUT: Mutex<InputState> = Mutex::new(InputState::new());

pub fn init() {
    flush_output();
    unsafe {
        if wait_input_empty() {
            outb(DATA_PORT, 0xF4);
        }
        if wait_output_full() {
            let _ = inb(DATA_PORT);
        }
    }
}

pub fn poll_byte() -> Option<u8> {
    poll_input();
    INPUT.lock().tty.pop()
}

pub fn poll_event_byte() -> Option<u8> {
    poll_input();
    let mut input = INPUT.lock();
    let b = input.events.pop()?;
    if input.tty.peek() == Some(b) {
        let _ = input.tty.pop();
    }
    Some(b)
}

pub fn read_byte_blocking() -> u8 {
    loop {
        if let Some(b) = poll_byte() {
            return b;
        }
        spin_loop();
    }
}

fn poll_input() {
    let status = unsafe { inb(STATUS_PORT) };
    if status & STATUS_OUTPUT_FULL == 0 {
        return;
    }
    let scancode = unsafe { inb(DATA_PORT) };
    if status & STATUS_AUX_DATA != 0 {
        return;
    }
    let mut input = INPUT.lock();
    if let Some(b) = input.keyboard.decode(scancode) {
        input.push(b);
    }
}

fn flush_output() {
    for _ in 0..64 {
        if unsafe { inb(STATUS_PORT) } & STATUS_OUTPUT_FULL == 0 {
            break;
        }
        unsafe {
            let _ = inb(DATA_PORT);
        }
    }
}

fn wait_input_empty() -> bool {
    for _ in 0..100_000 {
        if unsafe { inb(STATUS_PORT) } & STATUS_INPUT_FULL == 0 {
            return true;
        }
        spin_loop();
    }
    false
}

fn wait_output_full() -> bool {
    for _ in 0..100_000 {
        if unsafe { inb(STATUS_PORT) } & STATUS_OUTPUT_FULL != 0 {
            return true;
        }
        spin_loop();
    }
    false
}

struct InputState {
    keyboard: Keyboard,
    tty: ByteQueue,
    events: ByteQueue,
}

impl InputState {
    const fn new() -> Self {
        Self {
            keyboard: Keyboard::new(),
            tty: ByteQueue::new(),
            events: ByteQueue::new(),
        }
    }

    fn push(&mut self, b: u8) {
        self.tty.push(b);
        self.events.push(b);
    }
}

struct ByteQueue {
    buf: [u8; INPUT_QUEUE],
    head: usize,
    len: usize,
}

impl ByteQueue {
    const fn new() -> Self {
        Self {
            buf: [0; INPUT_QUEUE],
            head: 0,
            len: 0,
        }
    }

    fn push(&mut self, b: u8) {
        if self.len == INPUT_QUEUE {
            self.buf[self.head] = b;
            self.head = (self.head + 1) % INPUT_QUEUE;
        } else {
            let idx = (self.head + self.len) % INPUT_QUEUE;
            self.buf[idx] = b;
            self.len += 1;
        }
    }

    fn pop(&mut self) -> Option<u8> {
        if self.len == 0 {
            return None;
        }
        let b = self.buf[self.head];
        self.head = (self.head + 1) % INPUT_QUEUE;
        self.len -= 1;
        Some(b)
    }

    fn peek(&self) -> Option<u8> {
        if self.len == 0 {
            return None;
        }
        Some(self.buf[self.head])
    }
}

struct Keyboard {
    shift: bool,
    caps: bool,
    extended: bool,
}

impl Keyboard {
    const fn new() -> Self {
        Self {
            shift: false,
            caps: false,
            extended: false,
        }
    }

    fn decode(&mut self, scancode: u8) -> Option<u8> {
        if scancode == 0xE0 {
            self.extended = true;
            return None;
        }
        if scancode == 0xE1 {
            return None;
        }

        let released = scancode & 0x80 != 0;
        let code = scancode & 0x7F;

        match code {
            0x2A | 0x36 => {
                self.shift = !released;
                return None;
            }
            0x3A if !released => {
                self.caps = !self.caps;
                return None;
            }
            _ if released => return None,
            _ => {}
        }

        if self.extended {
            self.extended = false;
            return None;
        }

        translate_set1(code, self.shift, self.caps)
    }
}

fn letter(b: u8, shift: bool, caps: bool) -> u8 {
    if shift ^ caps {
        b - 32
    } else {
        b
    }
}

fn shifted(normal: u8, shifted: u8, shift: bool) -> u8 {
    if shift {
        shifted
    } else {
        normal
    }
}

fn translate_set1(code: u8, shift: bool, caps: bool) -> Option<u8> {
    Some(match code {
        0x01 => 0x1B,
        0x02 => shifted(b'1', b'!', shift),
        0x03 => shifted(b'2', b'@', shift),
        0x04 => shifted(b'3', b'#', shift),
        0x05 => shifted(b'4', b'$', shift),
        0x06 => shifted(b'5', b'%', shift),
        0x07 => shifted(b'6', b'^', shift),
        0x08 => shifted(b'7', b'&', shift),
        0x09 => shifted(b'8', b'*', shift),
        0x0A => shifted(b'9', b'(', shift),
        0x0B => shifted(b'0', b')', shift),
        0x0C => shifted(b'-', b'_', shift),
        0x0D => shifted(b'=', b'+', shift),
        0x0E => 0x08,
        0x0F => b'\t',
        0x10 => letter(b'q', shift, caps),
        0x11 => letter(b'w', shift, caps),
        0x12 => letter(b'e', shift, caps),
        0x13 => letter(b'r', shift, caps),
        0x14 => letter(b't', shift, caps),
        0x15 => letter(b'y', shift, caps),
        0x16 => letter(b'u', shift, caps),
        0x17 => letter(b'i', shift, caps),
        0x18 => letter(b'o', shift, caps),
        0x19 => letter(b'p', shift, caps),
        0x1A => shifted(b'[', b'{', shift),
        0x1B => shifted(b']', b'}', shift),
        0x1C => b'\n',
        0x1E => letter(b'a', shift, caps),
        0x1F => letter(b's', shift, caps),
        0x20 => letter(b'd', shift, caps),
        0x21 => letter(b'f', shift, caps),
        0x22 => letter(b'g', shift, caps),
        0x23 => letter(b'h', shift, caps),
        0x24 => letter(b'j', shift, caps),
        0x25 => letter(b'k', shift, caps),
        0x26 => letter(b'l', shift, caps),
        0x27 => shifted(b';', b':', shift),
        0x28 => shifted(b'\'', b'"', shift),
        0x29 => shifted(b'`', b'~', shift),
        0x2B => shifted(b'\\', b'|', shift),
        0x2C => letter(b'z', shift, caps),
        0x2D => letter(b'x', shift, caps),
        0x2E => letter(b'c', shift, caps),
        0x2F => letter(b'v', shift, caps),
        0x30 => letter(b'b', shift, caps),
        0x31 => letter(b'n', shift, caps),
        0x32 => letter(b'm', shift, caps),
        0x33 => shifted(b',', b'<', shift),
        0x34 => shifted(b'.', b'>', shift),
        0x35 => shifted(b'/', b'?', shift),
        0x39 => b' ',
        _ => return None,
    })
}
