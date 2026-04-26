//! personaOS kernel — M1 milestone.
//!
//! Entry: `personaboot` hands us a `&BootInfo` in rdi. M1 brings up:
//!   - GDT + TSS (with IST for #DF, RSP0 for ring-3 returns)
//!   - IDT with every CPU exception and a LAPIC timer vector
//!   - Physical frame allocator over the bootloader memory map
//!   - 4 MiB kernel heap
//!   - VMM that walks CR3 via the HHDM
//!   - LAPIC timer @ ~100 Hz driving the scheduler
//!   - syscall/sysret MSRs programmed
//!   - One user task spawned into ring 3 that loops over write/yield/getpid
//!
//! After the init task is spawned, the kernel enters the scheduler and never
//! returns.

#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![forbid(unsafe_op_in_unsafe_fn)]

extern crate alloc;

mod arch;
mod drivers;
mod fs;
mod mm;
mod panic;
mod sched;
mod syscall;
mod user;

use core::fmt::Write as _;

use persona_shared::{BootInfo, BOOT_INFO_MAGIC, BOOT_INFO_VERSION};

use crate::drivers::framebuffer::FramebufferConsole;
use crate::drivers::serial::SerialPort;

/// # Safety
///
/// Must only be invoked once, by the bootloader, with a valid `BootInfo*`.
#[no_mangle]
#[link_section = ".text._start"]
pub unsafe extern "sysv64" fn _start(info: *const BootInfo) -> ! {
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

    // Interrupts off until everything is live.
    arch::x86_64::cli();

    unsafe {
        mm::init(&info);
    }
    let _ = writeln!(
        serial,
        "[kernel] PMM: {} frames total, {} free",
        mm::pmm::total_frames(),
        mm::pmm::free_frames(),
    );

    unsafe {
        arch::x86_64::init_bsp();
    }
    let _ = serial.write_str("[kernel] arch up: GDT/IDT/LAPIC/syscall\n");

    unsafe {
        arch::x86_64::acpi::init(info.rsdp_phys);
    }
    drivers::pci::enumerate();

    // Bring up the first NVMe controller, if any.
    let nvme = drivers::nvme::init_from_pci();

    // Mount the root: PondFS on NVMe if available, ramfs (with /init baked in)
    // otherwise. Ramfs also always lands on /boot as a scratch area.
    let rfs = fs::ramfs::RamFs::new();
    rfs.put_file("/hello.txt", b"hello from ramfs\n");
    rfs.put_file("/init", user::INIT_ELF);
    fs::mount("/boot", rfs.clone());
    let _ = serial.write_str("[vfs] mounted ramfs at /boot\n");

    let mounted_pondfs = if let Some(dev) = nvme.clone() {
        match fs::pondfs::PondFs::mount(dev) {
            Ok(pfs) => {
                fs::mount("/", pfs);
                let _ = serial.write_str("[vfs] mounted pondfs at /\n");
                true
            }
            Err(e) => {
                let _ = writeln!(serial, "[vfs] pondfs mount failed: {:?}", e);
                false
            }
        }
    } else {
        false
    };

    if !mounted_pondfs {
        // Fall back to ramfs on / so early boot still works without NVMe.
        fs::mount("/", rfs);
        let _ = serial.write_str("[vfs] mounted ramfs at / (fallback)\n");
        user::seed_init_into_vfs();
    }

    // Paint the framebuffer so the screen has the M1 banner.
    let mut fb = unsafe { FramebufferConsole::new(info.framebuffer) };
    fb.clear(0x0B1020);
    fb.draw_string(32, 32, "personaOS booted", 0xE6E6FA);
    fb.draw_string(32, 56, "M1 - kernel: sched + ring-3 + syscalls", 0x9FB4FF);

    // Idle task so the run queue is never empty.
    let idle = sched::spawn_idle(idle_task);
    sched::enqueue(idle);

    // First user task.
    let init = user::spawn_init();
    let _ = writeln!(serial, "[kernel] spawning init pid={}", init.id());
    sched::enqueue(init);

    let _ = serial.write_str("[kernel] M1 milestone reached, entering scheduler.\n");
    // NOTE: interrupts stay OFF until idle_task enables them. Turning them
    // on before the very first context-bootstrap asm means a timer tick
    // can fire inside `run()` and leave IF=0 after the `ret` into idle.
    sched::run();
}

extern "C" fn idle_task() -> ! {
    arch::x86_64::sti();
    loop {
        arch::x86_64::halt();
    }
}
