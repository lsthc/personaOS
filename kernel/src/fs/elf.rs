//! ELF loader for ring-3 binaries.
//!
//! We only parse 64-bit little-endian static executables. Each `PT_LOAD`
//! segment is mapped into a fresh user AddressSpace, zero-filled, and then
//! the file bytes are copied in. Returns the entry point so the caller can
//! build an IRETQ frame.

use xmas_elf::program::{ProgramHeader, Type as PhType};
use xmas_elf::ElfFile;

use crate::mm::vmm::{AddressSpace, MapFlags};
use crate::mm::PAGE_SIZE;

#[derive(Debug)]
#[allow(dead_code)]
pub enum ElfError {
    Parse,
    Map,
    Copy,
}

/// Load `bytes` as an ELF into `address_space`. Returns the entry point
/// virtual address.
///
/// # Safety
/// Must be called in kernel mode before the target task is runnable.
pub unsafe fn load(bytes: &[u8], address_space: &AddressSpace) -> Result<u64, ElfError> {
    let elf = ElfFile::new(bytes).map_err(|_| ElfError::Parse)?;
    let entry = elf.header.pt2.entry_point();

    // First pass: map and copy each PT_LOAD.
    for ph in elf.program_iter() {
        if ph.get_type().ok() != Some(PhType::Load) {
            continue;
        }
        match ph {
            ProgramHeader::Ph64(p) => unsafe {
                load_seg(
                    &elf,
                    p.virtual_addr,
                    p.offset,
                    p.file_size,
                    p.mem_size,
                    p.flags.0,
                    address_space,
                )?
            },
            ProgramHeader::Ph32(_) => return Err(ElfError::Parse),
        }
    }
    Ok(entry)
}

#[allow(clippy::too_many_arguments)]
unsafe fn load_seg(
    elf: &ElfFile,
    virt: u64,
    file_off: u64,
    file_size: u64,
    mem_size: u64,
    flags: u32,
    as_: &AddressSpace,
) -> Result<(), ElfError> {
    let aligned_virt = virt & !(PAGE_SIZE as u64 - 1);
    let page_off = (virt - aligned_virt) as usize;
    let pages = (page_off + mem_size as usize).div_ceil(PAGE_SIZE);

    let mut map_flags = MapFlags::USER;
    // ELF segment flag bits: 1 = exec, 2 = write, 4 = read.
    if flags & 0x2 != 0 {
        map_flags |= MapFlags::WRITE;
    }
    if flags & 0x1 == 0 {
        map_flags |= MapFlags::NX;
    }

    as_.map_anon(aligned_virt, pages, map_flags | MapFlags::WRITE)
        .map_err(|_| ElfError::Map)?;

    as_.zero_user(virt, mem_size as usize)
        .map_err(|_| ElfError::Copy)?;
    let start = file_off as usize;
    let end = start + file_size as usize;
    as_.copy_to_user(virt, &elf.input[start..end])
        .map_err(|_| ElfError::Copy)?;

    Ok(())
}
