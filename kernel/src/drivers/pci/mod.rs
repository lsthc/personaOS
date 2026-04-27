//! PCI(e) enumeration over ECAM (MCFG).
//!
//! Legacy 0xCF8/0xCFC PIO is intentionally not supported — on QEMU q35
//! MCFG is always present. We walk bus/device/function, read the config
//! header, probe BAR sizes, and store a flat list of devices other
//! subsystems can consult.

mod config;

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt::Write as _;

use spin::Mutex;

use crate::arch::x86_64::acpi;
use crate::drivers::serial::SerialPort;

pub use config::Device;

static DEVICES: Mutex<Vec<Arc<Device>>> = Mutex::new(Vec::new());

/// Enumerate every function reachable via each MCFG allocation. Safe to call
/// once from kernel init, after ACPI is parsed.
pub fn enumerate() {
    let mut serial = unsafe { SerialPort::new(0x3F8) };
    let mcfg = match acpi::mcfg() {
        Some(m) => m,
        None => {
            let _ = serial.write_str("[pci] no MCFG, skipping enumeration\n");
            return;
        }
    };

    let mut devs = DEVICES.lock();
    for alloc in mcfg.allocations {
        let base = alloc.base;
        let segment = alloc.segment;
        let bus_start = alloc.bus_start;
        let bus_end = alloc.bus_end;
        let _ = writeln!(
            serial,
            "[pci] ecam seg={} base={:#x} bus={:#x}..={:#x}",
            segment, base, bus_start, bus_end
        );
        for bus in bus_start..=bus_end {
            for dev in 0u8..32 {
                for func in 0u8..8 {
                    if let Some(d) = config::probe(base, bus, dev, func) {
                        let _ = writeln!(
                            &mut serial,
                            "[pci] {:02x}:{:02x}.{}  {:04x}:{:04x}  class {:02x}.{:02x}.{:02x}  bars={}",
                            d.bus, d.device, d.function,
                            d.vendor_id, d.device_id,
                            d.class, d.subclass, d.prog_if,
                            d.active_bar_count(),
                        );
                        for (i, bar) in d.bars.iter().enumerate() {
                            if let Some(b) = bar {
                                let _ = writeln!(
                                    &mut serial,
                                    "[pci]   BAR{} {} addr={:#x} size={:#x}",
                                    i,
                                    if b.io { "io  " } else { "mmio" },
                                    b.base,
                                    b.size,
                                );
                            }
                        }
                        if d.msix.is_some() {
                            let _ = writeln!(&mut serial, "[pci]   MSI-X supported");
                        }
                        devs.push(Arc::new(d));
                    } else if func == 0 {
                        // Function 0 absent → skip the whole device.
                        break;
                    }
                }
            }
        }
    }
}

/// Snapshot of every enumerated device. Returns clones of the `Arc`s so the
/// caller can release the lock immediately.
#[allow(dead_code)] // consumed by drivers not yet wired up
pub fn all() -> Vec<Arc<Device>> {
    DEVICES.lock().clone()
}

/// Find the first device matching a class/subclass/prog-if triple. Used by
/// the NVMe / xHCI drivers to locate their controller.
#[allow(dead_code)] // used by M2.3 / M2.5
pub fn find_class(class: u8, subclass: u8, prog_if: u8) -> Option<Arc<Device>> {
    DEVICES
        .lock()
        .iter()
        .find(|d| d.class == class && d.subclass == subclass && d.prog_if == prog_if)
        .cloned()
}
