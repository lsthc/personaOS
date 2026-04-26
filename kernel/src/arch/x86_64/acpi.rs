//! ACPI discovery — just enough to find MCFG for PCIe ECAM.
//!
//! We do not parse AML. All we need right now is:
//!   1. Validate the RSDP the bootloader handed us.
//!   2. Walk XSDT (ACPI 2+) for known table signatures.
//!   3. Pull the MCFG allocation list so PCI enumeration can reach config
//!      space over MMIO.
//!
//! Tables live in memory the firmware classified as `AcpiReclaimable` /
//! `AcpiNvs`; both remain reachable through the HHDM.

use core::mem;
use core::ptr;

use persona_shared::HHDM_OFFSET;
use spin::Once;

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct RsdpV2 {
    signature: [u8; 8],
    _checksum: u8,
    _oem_id: [u8; 6],
    revision: u8,
    _rsdt_address: u32,
    _length: u32,
    xsdt_address: u64,
    _ext_checksum: u8,
    _reserved: [u8; 3],
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct SdtHeader {
    signature: [u8; 4],
    length: u32,
    _revision: u8,
    _checksum: u8,
    _oem_id: [u8; 6],
    _oem_table_id: [u8; 8],
    _oem_revision: u32,
    _creator_id: u32,
    _creator_revision: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct McfgAllocation {
    pub base: u64,
    pub segment: u16,
    pub bus_start: u8,
    pub bus_end: u8,
    _reserved: u32,
}

pub struct Mcfg {
    pub allocations: &'static [McfgAllocation],
}

static MCFG: Once<Option<Mcfg>> = Once::new();

/// Locate ACPI tables starting from `rsdp_phys` (from the UEFI configuration
/// table, as shipped in `BootInfo`). Parses MCFG if present. Silently tolerates
/// missing tables — the caller decides what to do if MCFG is absent.
///
/// # Safety
/// Caller must pass the same physical RSDP pointer the bootloader stored;
/// must run after HHDM is live (it always is by kernel entry).
pub unsafe fn init(rsdp_phys: u64) {
    if rsdp_phys == 0 {
        MCFG.call_once(|| None);
        return;
    }
    let rsdp = unsafe { &*((rsdp_phys + HHDM_OFFSET) as *const RsdpV2) };
    if &rsdp.signature != b"RSD PTR " {
        MCFG.call_once(|| None);
        return;
    }
    if rsdp.revision < 2 {
        // ACPI 1.0-only system. We're not supporting it.
        MCFG.call_once(|| None);
        return;
    }
    let xsdt_phys = rsdp.xsdt_address;
    let xsdt = unsafe { &*((xsdt_phys + HHDM_OFFSET) as *const SdtHeader) };
    if &xsdt.signature != b"XSDT" {
        MCFG.call_once(|| None);
        return;
    }
    let entries_bytes = xsdt.length as usize - mem::size_of::<SdtHeader>();
    let entries = entries_bytes / 8;
    let entry_ptr =
        (xsdt_phys + HHDM_OFFSET + mem::size_of::<SdtHeader>() as u64) as *const u64;

    let mut mcfg_found: Option<Mcfg> = None;
    for i in 0..entries {
        let table_phys = unsafe { ptr::read_unaligned(entry_ptr.add(i)) };
        let hdr = unsafe { &*((table_phys + HHDM_OFFSET) as *const SdtHeader) };
        if &hdr.signature == b"MCFG" {
            mcfg_found = Some(parse_mcfg(table_phys, hdr.length as usize));
            break;
        }
    }
    MCFG.call_once(|| mcfg_found);
}

fn parse_mcfg(table_phys: u64, length: usize) -> Mcfg {
    // MCFG header: standard SDT (36 bytes) + 8 bytes reserved = 44 before
    // the allocation array.
    const MCFG_ALLOC_OFF: usize = mem::size_of::<SdtHeader>() + 8;
    let data_bytes = length.saturating_sub(MCFG_ALLOC_OFF);
    let count = data_bytes / mem::size_of::<McfgAllocation>();
    let ptr = (table_phys + HHDM_OFFSET + MCFG_ALLOC_OFF as u64) as *const McfgAllocation;
    let allocations = unsafe { core::slice::from_raw_parts(ptr, count) };
    Mcfg { allocations }
}

pub fn mcfg() -> Option<&'static Mcfg> {
    MCFG.get().and_then(|m| m.as_ref())
}
