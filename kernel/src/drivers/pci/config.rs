//! PCI config-space access via ECAM and the device / BAR / capability types
//! the rest of the kernel consumes.

use persona_shared::HHDM_OFFSET;

/// ECAM offset for (bus, dev, func). Each function gets 4 KiB.
fn ecam_addr(base: u64, bus: u8, dev: u8, func: u8) -> u64 {
    base + ((bus as u64) << 20) + ((dev as u64) << 15) + ((func as u64) << 12)
}

#[allow(clippy::manual_range_patterns)]
fn cfg_ptr(base: u64, bus: u8, dev: u8, func: u8, off: u16) -> *mut u32 {
    ((ecam_addr(base, bus, dev, func) + off as u64) + HHDM_OFFSET) as *mut u32
}

fn read_u32(base: u64, bus: u8, dev: u8, func: u8, off: u16) -> u32 {
    unsafe { cfg_ptr(base, bus, dev, func, off).read_volatile() }
}

fn write_u32(base: u64, bus: u8, dev: u8, func: u8, off: u16, v: u32) {
    unsafe { cfg_ptr(base, bus, dev, func, off).write_volatile(v) }
}

fn read_u16(base: u64, bus: u8, dev: u8, func: u8, off: u16) -> u16 {
    let w = read_u32(base, bus, dev, func, off & !3);
    (w >> ((off & 3) * 8)) as u16
}

fn read_u8(base: u64, bus: u8, dev: u8, func: u8, off: u16) -> u8 {
    let w = read_u32(base, bus, dev, func, off & !3);
    (w >> ((off & 3) * 8)) as u8
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // prefetchable is read once drivers start caring about WC mappings
pub struct Bar {
    pub base: u64,
    pub size: u64,
    pub io: bool,
    pub prefetchable: bool,
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // every field is consumed by nvme/xhci MSI-X programming
pub struct MsixCap {
    pub cap_off: u16,
    pub table_size: u16,
    pub table_bir: u8,
    pub table_off: u32,
    pub pba_bir: u8,
    pub pba_off: u32,
}

#[derive(Debug)]
#[allow(dead_code)] // segment/header_type read once multi-segment or PCI-to-PCI bridge support lands
pub struct Device {
    pub segment: u16,
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub ecam_base: u64,

    pub vendor_id: u16,
    pub device_id: u16,
    pub class: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub header_type: u8,

    pub bars: [Option<Bar>; 6],
    pub msix: Option<MsixCap>,
}

impl Device {
    pub fn active_bar_count(&self) -> usize {
        self.bars.iter().filter(|b| b.is_some()).count()
    }

    /// Raw 32-bit config read at `off`. Useful for drivers touching vendor
    /// registers we haven't parsed into structs.
    #[allow(dead_code)]
    pub fn read_u32(&self, off: u16) -> u32 {
        read_u32(self.ecam_base, self.bus, self.device, self.function, off)
    }

    /// Raw 32-bit config write at `off`.
    #[allow(dead_code)]
    pub fn write_u32(&self, off: u16, v: u32) {
        write_u32(self.ecam_base, self.bus, self.device, self.function, off, v);
    }

    /// Enable MMIO + bus-master for this device. Almost every driver needs
    /// both before touching BARs.
    #[allow(dead_code)]
    pub fn enable_mmio_bus_master(&self) {
        let mut cmd = self.read_u32(0x04) & 0xFFFF;
        cmd |= 0x0002 /* MEM_SPACE */ | 0x0004 /* BUS_MASTER */;
        let status_hi = self.read_u32(0x04) & 0xFFFF_0000;
        self.write_u32(0x04, status_hi | cmd);
    }

    /// Enable I/O port space + bus-master for legacy PCI devices such as AC97.
    #[allow(dead_code)]
    pub fn enable_io_bus_master(&self) {
        let mut cmd = self.read_u32(0x04) & 0xFFFF;
        cmd |= 0x0001 /* IO_SPACE */ | 0x0004 /* BUS_MASTER */;
        let status_hi = self.read_u32(0x04) & 0xFFFF_0000;
        self.write_u32(0x04, status_hi | cmd);
    }
}

/// Probe one (bus, dev, func). Returns `Some(device)` iff a function is
/// present; otherwise `None` to let the caller short-circuit.
pub fn probe(base: u64, bus: u8, dev: u8, func: u8) -> Option<Device> {
    let vendor = read_u16(base, bus, dev, func, 0);
    if vendor == 0xFFFF {
        return None;
    }
    let device_id = read_u16(base, bus, dev, func, 2);
    let class = read_u8(base, bus, dev, func, 0x0B);
    let subclass = read_u8(base, bus, dev, func, 0x0A);
    let prog_if = read_u8(base, bus, dev, func, 0x09);
    let header_type = read_u8(base, bus, dev, func, 0x0E);

    let mut bars = [None; 6];
    // Only parse BARs for header type 0 (endpoint devices). Type 1 is a
    // PCI-to-PCI bridge which M2 doesn't need to drive.
    if header_type & 0x7F == 0 {
        let mut i = 0usize;
        while i < 6 {
            match read_bar(base, bus, dev, func, i) {
                (Some(bar), next) => {
                    bars[i] = Some(bar);
                    i = next;
                }
                (None, next) => i = next,
            }
        }
    }

    let msix = find_msix(base, bus, dev, func);

    Some(Device {
        segment: 0,
        bus,
        device: dev,
        function: func,
        ecam_base: base,
        vendor_id: vendor,
        device_id,
        class,
        subclass,
        prog_if,
        header_type,
        bars,
        msix,
    })
}

/// Read BAR `index`, probing its size via the classic write-all-ones trick.
/// Returns the parsed BAR (if any) and the next index to consume: 64-bit BARs
/// take two slots.
fn read_bar(base: u64, bus: u8, dev: u8, func: u8, index: usize) -> (Option<Bar>, usize) {
    let off = 0x10 + (index as u16) * 4;
    let orig = read_u32(base, bus, dev, func, off);
    if orig == 0 {
        return (None, index + 1);
    }
    let io = orig & 1 == 1;
    if io {
        write_u32(base, bus, dev, func, off, 0xFFFF_FFFC);
        let probed = read_u32(base, bus, dev, func, off) & 0xFFFF_FFFC;
        write_u32(base, bus, dev, func, off, orig);
        let size = (!probed).wrapping_add(1) as u64;
        return (
            Some(Bar {
                base: (orig & 0xFFFF_FFFC) as u64,
                size,
                io: true,
                prefetchable: false,
            }),
            index + 1,
        );
    }
    let ty = (orig >> 1) & 0x3;
    let prefetchable = orig & 0x08 != 0;
    if ty == 0x2 && index < 5 {
        // 64-bit BAR spans two config slots.
        let hi_off = off + 4;
        let orig_hi = read_u32(base, bus, dev, func, hi_off);
        write_u32(base, bus, dev, func, off, 0xFFFF_FFF0);
        write_u32(base, bus, dev, func, hi_off, 0xFFFF_FFFF);
        let lo = read_u32(base, bus, dev, func, off) & 0xFFFF_FFF0;
        let hi = read_u32(base, bus, dev, func, hi_off);
        write_u32(base, bus, dev, func, off, orig);
        write_u32(base, bus, dev, func, hi_off, orig_hi);
        let masked = ((hi as u64) << 32) | lo as u64;
        let size = (!masked).wrapping_add(1);
        let addr = ((orig_hi as u64) << 32) | (orig & 0xFFFF_FFF0) as u64;
        (
            Some(Bar {
                base: addr,
                size,
                io: false,
                prefetchable,
            }),
            index + 2,
        )
    } else {
        // 32-bit memory BAR.
        write_u32(base, bus, dev, func, off, 0xFFFF_FFF0);
        let probed = read_u32(base, bus, dev, func, off) & 0xFFFF_FFF0;
        write_u32(base, bus, dev, func, off, orig);
        let size = (!(probed as u64)).wrapping_add(1) & 0xFFFF_FFFF;
        (
            Some(Bar {
                base: (orig & 0xFFFF_FFF0) as u64,
                size,
                io: false,
                prefetchable,
            }),
            index + 1,
        )
    }
}

/// Walk the capability chain starting at the pointer in config offset 0x34
/// and return any MSI-X capability we find.
fn find_msix(base: u64, bus: u8, dev: u8, func: u8) -> Option<MsixCap> {
    let status = read_u16(base, bus, dev, func, 0x06);
    if status & (1 << 4) == 0 {
        return None; // no capabilities list
    }
    let mut cap = read_u8(base, bus, dev, func, 0x34) & 0xFC;
    let mut hops = 0;
    while cap != 0 && hops < 48 {
        let id = read_u8(base, bus, dev, func, cap as u16);
        let next = read_u8(base, bus, dev, func, cap as u16 + 1);
        if id == 0x11 {
            // MSI-X capability layout:
            //   +0: ID (0x11) / +1: next / +2: Message Control (u16)
            //   +4: Table BIR/offset (u32)
            //   +8: PBA BIR/offset (u32)
            let msg_ctrl = read_u16(base, bus, dev, func, cap as u16 + 2);
            let table = read_u32(base, bus, dev, func, cap as u16 + 4);
            let pba = read_u32(base, bus, dev, func, cap as u16 + 8);
            return Some(MsixCap {
                cap_off: cap as u16,
                table_size: (msg_ctrl & 0x7FF) + 1,
                table_bir: (table & 0x7) as u8,
                table_off: table & !0x7,
                pba_bir: (pba & 0x7) as u8,
                pba_off: pba & !0x7,
            });
        }
        cap = next & 0xFC;
        hops += 1;
    }
    None
}
