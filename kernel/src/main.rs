//! personaOS kernel — M0 milestone.
//!
//! Entry point receives a `&BootInfo` from `personaboot` in `rdi`. For M0
//! the kernel simply initializes a serial console, draws a confirmation
//! message to the framebuffer, and halts.

#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![forbid(unsafe_op_in_unsafe_fn)]

mod arch;
mod drivers;
mod panic;

use core::sync::atomic::{AtomicBool, Ordering};

use persona_shared::{BootInfo, BOOT_INFO_MAGIC, BOOT_INFO_VERSION};

use crate::drivers::framebuffer::FramebufferConsole;
use crate::drivers::serial::SerialPort;

static BOOTED: AtomicBool = AtomicBool::new(false);

/// Kernel entry. Called by `personaboot` with a pointer to `BootInfo` in RDI
/// and a 64 KiB stack already set up in the higher half.
///
/// # Safety
///
/// Must only be invoked once, by the bootloader, with a valid `BootInfo*`.
#[no_mangle]
pub unsafe extern "sysv64" fn _start(info: *const BootInfo) -> ! {
    // Read the BootInfo. We copy it out of bootloader memory into the kernel
    // image so that bootloader-reclaimable memory can later be freed.
    let info = unsafe { info.read() };

    let mut serial = unsafe { SerialPort::new(0x3F8) };
    serial.init();
    let _ = serial.write_str("[kernel] _start reached\n");

    if info.magic != BOOT_INFO_MAGIC {
        let _ = serial.write_str("[kernel] PANIC: bad BootInfo magic\n");
        loop {
            arch::x86_64::halt();
        }
    }
    if info.version != BOOT_INFO_VERSION {
        let _ = serial.write_str("[kernel] PANIC: BootInfo version mismatch\n");
        loop {
            arch::x86_64::halt();
        }
    }

    let _ = serial.write_str("[kernel] BootInfo OK\n");

    // Paint the framebuffer.
    let mut fb = unsafe { FramebufferConsole::new(info.framebuffer) };
    fb.clear(0x0B1020); // dark navy
    fb.draw_string(32, 32, "personaOS booted", 0xE6E6FA);
    fb.draw_string(
        32,
        56,
        "M0 — kernel entry reached, framebuffer online",
        0x9FB4FF,
    );

    BOOTED.store(true, Ordering::SeqCst);
    let _ = serial.write_str("[kernel] M0 milestone reached, halting.\n");

    loop {
        arch::x86_64::halt();
    }
}
