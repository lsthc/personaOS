//! Minimal VirtIO-GPU discovery.
//!
//! M4 still renders through the boot framebuffer. This module only identifies a
//! VirtIO-GPU PCI function so the graphics stack can report whether the future
//! accelerated backend is present.

use core::fmt::Write as _;

use crate::drivers::pci::Device;
use crate::drivers::serial::SerialPort;

pub struct GpuInfo {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub bar0_base: u64,
    pub bar0_size: u64,
}

const VIRTIO_VENDOR: u16 = 0x1AF4;
const VIRTIO_GPU_LEGACY: u16 = 0x1050;
const VIRTIO_GPU_TRANSITIONAL: u16 = 0x1000;

pub fn discover() -> Option<GpuInfo> {
    let mut serial = unsafe { SerialPort::new(0x3F8) };
    for dev in crate::drivers::pci::all() {
        if !is_virtio_gpu(&dev) {
            continue;
        }
        let Some(bar0) = dev.bars[0] else {
            let _ = writeln!(serial, "[virtio-gpu] candidate has no BAR0");
            return None;
        };
        if bar0.io {
            let _ = writeln!(serial, "[virtio-gpu] candidate BAR0 is I/O, unsupported");
            return None;
        }
        let _ = writeln!(
            serial,
            "[virtio-gpu] found at {:02x}:{:02x}.{} BAR0={:#x} size={:#x}",
            dev.bus, dev.device, dev.function, bar0.base, bar0.size,
        );
        return Some(GpuInfo {
            bus: dev.bus,
            device: dev.device,
            function: dev.function,
            bar0_base: bar0.base,
            bar0_size: bar0.size,
        });
    }
    let _ = writeln!(serial, "[virtio-gpu] not present; using boot framebuffer");
    None
}

fn is_virtio_gpu(dev: &Device) -> bool {
    if dev.vendor_id != VIRTIO_VENDOR {
        return false;
    }
    dev.device_id == VIRTIO_GPU_LEGACY || dev.device_id == VIRTIO_GPU_TRANSITIONAL
}
