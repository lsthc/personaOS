//! Kernel panic handler.

use core::panic::PanicInfo;

use crate::arch::x86_64::halt;
use crate::drivers::serial::SerialPort;

#[panic_handler]
fn on_panic(info: &PanicInfo) -> ! {
    // Best-effort serial write. Reinitialising is safe because the UART is
    // a well-known fixed I/O port and we are halting after this.
    let mut serial = unsafe { SerialPort::new(0x3F8) };
    serial.init();
    let _ = serial.write_str("\n[kernel] PANIC: ");
    if let Some(loc) = info.location() {
        let _ = serial.write_str(loc.file());
        let _ = serial.write_str(":");
        let _ = serial.write_dec_u32(loc.line());
        let _ = serial.write_str(" -> ");
    }
    let _ = serial.write_fmt_args(info.message());
    let _ = serial.write_str("\n");
    loop {
        halt();
    }
}
