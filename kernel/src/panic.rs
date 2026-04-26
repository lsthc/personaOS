//! Kernel panic handler.

use core::fmt::Write as _;
use core::panic::PanicInfo;

use crate::arch::x86_64::halt;
use crate::drivers::serial::SerialPort;

#[panic_handler]
fn on_panic(info: &PanicInfo) -> ! {
    // Best-effort serial write. Reinitialising is safe because the UART is
    // a well-known fixed I/O port and we are halting after this.
    let mut serial = unsafe { SerialPort::new(0x3F8) };
    serial.init();
    let _ = write!(serial, "\n[kernel] PANIC: {info}\n");
    loop {
        halt();
    }
}
