//! personaboot — UEFI bootloader for personaOS.
//!
//! Responsibilities:
//!   1. Obtain a linear framebuffer via the Graphics Output Protocol (GOP).
//!   2. Read `\EFI\personaOS\kernel.elf` from the EFI System Partition (ESP).
//!   3. Parse the ELF and allocate physical memory for each PT_LOAD segment.
//!   4. Build fresh page tables: identity map the low 4 GiB (for firmware),
//!      map all physical memory at the higher-half direct map (HHDM), and
//!      map kernel segments at their linked virtual addresses.
//!   5. Locate the ACPI RSDP in the UEFI configuration table.
//!   6. Construct a `BootInfo` struct.
//!   7. Call `ExitBootServices`, install the new CR3, switch to the kernel
//!      stack, and jump to the kernel entry point.

#![no_main]
#![no_std]
#![feature(abi_efiapi)]

extern crate alloc;

use alloc::vec::Vec;
use core::arch::asm;
use core::ptr;
use core::slice;

use persona_shared::{
    BootInfo, Framebuffer, MemoryKind, MemoryMap, MemoryRegion, PixelFormat, BOOT_INFO_MAGIC,
    BOOT_INFO_VERSION, HHDM_OFFSET,
};
use uefi::prelude::*;
use uefi::proto::console::gop::{GraphicsOutput, PixelFormat as GopPixelFormat};
use uefi::proto::media::file::{File, FileAttribute, FileInfo, FileMode, RegularFile};
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::table::boot::{AllocateType, MemoryType};
use uefi::table::cfg::{ACPI2_GUID, ACPI_GUID};
use uefi::CStr16;
use x86_64::registers::control::{Cr0, Cr0Flags, Cr3, Cr3Flags, Cr4, Cr4Flags};
use x86_64::structures::paging::{
    page_table::PageTableFlags as PTF, PageTable, PhysFrame, Size4KiB,
};
use x86_64::{PhysAddr, VirtAddr};
use xmas_elf::program::{ProgramHeader, Type as PhType};
use xmas_elf::ElfFile;

const KERNEL_PATH: &str = "\\EFI\\personaOS\\kernel.elf";
const KERNEL_STACK_SIZE: usize = 64 * 1024;
/// Identity-map the first 4 GiB so that firmware / MMIO remains reachable
/// immediately after switching to our page tables. Production will trim this.
const IDENTITY_MAP_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[entry]
fn efi_main(image: Handle, mut st: SystemTable<Boot>) -> Status {
    uefi_services::init(&mut st).expect("uefi_services init");
    log::info!("[personaboot] hello, world");

    let fb = init_framebuffer(st.boot_services());
    log::info!(
        "[personaboot] framebuffer {}x{} pitch={} bpp={}",
        fb.width,
        fb.height,
        fb.pitch,
        fb.bits_per_pixel
    );

    let kernel_bytes = load_kernel(image, st.boot_services());
    log::info!("[personaboot] loaded kernel.elf: {} bytes", kernel_bytes.len());

    let elf = ElfFile::new(&kernel_bytes).expect("parse kernel ELF");
    let entry_point = elf.header.pt2.entry_point();
    log::info!("[personaboot] kernel entry = {:#x}", entry_point);

    let stack_phys = alloc_pages(st.boot_services(), pages_for(KERNEL_STACK_SIZE as u64));
    let stack_top_virt = HHDM_OFFSET + stack_phys + KERNEL_STACK_SIZE as u64;
    log::info!("[personaboot] kernel stack top = {:#x}", stack_top_virt);

    let rsdp = find_rsdp(&st);
    log::info!("[personaboot] rsdp = {:#x}", rsdp);

    // Build page tables BEFORE exiting boot services (we need the allocator).
    let (pml4_phys, _loaded) = build_page_tables(st.boot_services(), &elf, &fb);
    log::info!("[personaboot] pml4 @ {:#x}", pml4_phys);

    // Allocate and fill the BootInfo + memory-map storage in boot-services
    // memory that we'll mark as BootloaderReclaimable.
    let boot_info_phys = alloc_pages(st.boot_services(), 1);
    let mmap_entries_phys = alloc_pages(st.boot_services(), 16); // up to ~32k entries

    // Exit boot services. From here on, no more prints or allocations.
    let (_runtime_st, mmap) = st.exit_boot_services(MemoryType::LOADER_DATA);

    // Convert UEFI memory map to our format.
    let mmap_ptr = (mmap_entries_phys + HHDM_OFFSET) as *mut MemoryRegion;
    let mut count = 0usize;
    for desc in mmap.entries() {
        let kind = match desc.ty {
            MemoryType::CONVENTIONAL => MemoryKind::Usable,
            MemoryType::BOOT_SERVICES_CODE | MemoryType::BOOT_SERVICES_DATA => {
                MemoryKind::BootloaderReclaimable
            }
            MemoryType::LOADER_CODE | MemoryType::LOADER_DATA => MemoryKind::KernelAndModules,
            MemoryType::ACPI_RECLAIM => MemoryKind::AcpiReclaimable,
            MemoryType::ACPI_NON_VOLATILE => MemoryKind::AcpiNvs,
            MemoryType::UNUSABLE => MemoryKind::BadMemory,
            _ => MemoryKind::Reserved,
        };
        unsafe {
            ptr::write(
                mmap_ptr.add(count),
                MemoryRegion {
                    base: desc.phys_start,
                    length: desc.page_count * 4096,
                    kind,
                },
            );
        }
        count += 1;
    }

    // Fill BootInfo.
    let info = BootInfo {
        magic: BOOT_INFO_MAGIC,
        version: BOOT_INFO_VERSION,
        _pad0: 0,
        framebuffer: fb,
        memory_map: MemoryMap {
            entries: mmap_ptr as *const _,
            count,
        },
        rsdp_phys: rsdp,
        cmdline_ptr: ptr::null(),
        cmdline_len: 0,
        hhdm_offset: HHDM_OFFSET,
    };
    let info_ptr = (boot_info_phys + HHDM_OFFSET) as *mut BootInfo;
    unsafe { ptr::write(info_ptr, info) };

    // Switch CR3 and jump into the kernel. The kernel receives &BootInfo in
    // rdi per the SysV AMD64 ABI.
    unsafe {
        Cr3::write(
            PhysFrame::from_start_address(PhysAddr::new(pml4_phys)).unwrap(),
            Cr3Flags::empty(),
        );

        asm!(
            "mov rsp, {stack}",
            "mov rbp, 0",
            "jmp {entry}",
            stack = in(reg) stack_top_virt,
            entry = in(reg) entry_point,
            in("rdi") info_ptr as u64,
            options(noreturn)
        );
    }
}

// ---------------------------------------------------------------------------
// Framebuffer
// ---------------------------------------------------------------------------

fn init_framebuffer(bt: &BootServices) -> Framebuffer {
    let handle = bt
        .get_handle_for_protocol::<GraphicsOutput>()
        .expect("locate GOP handle");
    let mut gop = bt
        .open_protocol_exclusive::<GraphicsOutput>(handle)
        .expect("open GOP");

    // Pick the largest mode that has an RGB/BGR pixel format.
    let mut best: Option<(usize, (u32, u32), GopPixelFormat)> = None;
    for (idx, mode) in gop.modes(bt).enumerate() {
        let info = mode.info();
        let (w, h) = info.resolution();
        let pf = info.pixel_format();
        if !matches!(pf, GopPixelFormat::Rgb | GopPixelFormat::Bgr) {
            continue;
        }
        let px = w as u64 * h as u64;
        if best.map_or(true, |(_, (bw, bh), _)| (bw as u64 * bh as u64) < px) {
            best = Some((idx, (w as u32, h as u32), pf));
        }
    }
    if let Some((idx, _, _)) = best {
        let mode = gop.modes(bt).nth(idx).unwrap();
        gop.set_mode(&mode).expect("set GOP mode");
    }

    let info = gop.current_mode_info();
    let (w, h) = info.resolution();
    let pitch = info.stride() as u32 * 4;
    let pixel_format = match info.pixel_format() {
        GopPixelFormat::Rgb => PixelFormat::Rgbx8888,
        GopPixelFormat::Bgr => PixelFormat::Bgrx8888,
        _ => PixelFormat::Unknown,
    };
    let mut fb_buf = gop.frame_buffer();
    let base_phys = fb_buf.as_mut_ptr() as u64;
    // The kernel accesses the framebuffer through the HHDM; convert now.
    let base_virt = (base_phys + HHDM_OFFSET) as *mut u8;

    Framebuffer {
        base: base_virt,
        width: w as u32,
        height: h as u32,
        pitch,
        bits_per_pixel: 32,
        pixel_format,
    }
}

// ---------------------------------------------------------------------------
// Kernel loading
// ---------------------------------------------------------------------------

fn load_kernel(image: Handle, bt: &BootServices) -> Vec<u8> {
    let mut sfs = bt
        .get_image_file_system(image)
        .expect("open image filesystem");
    let mut root = sfs.open_volume().expect("open ESP root");

    // Convert the path to CStr16.
    let mut buf = [0u16; 64];
    let path = CStr16::from_str_with_buf(KERNEL_PATH, &mut buf).expect("kernel path -> CStr16");

    let file = root
        .open(path, FileMode::Read, FileAttribute::empty())
        .expect("open kernel.elf");
    let mut file: RegularFile = file.into_regular_file().expect("kernel.elf is a regular file");

    // Query size.
    let mut info_buf = [0u8; 512];
    let info: &mut FileInfo = file
        .get_info::<FileInfo>(&mut info_buf)
        .expect("get FileInfo");
    let size = info.file_size() as usize;

    let mut bytes = alloc::vec![0u8; size];
    let read = file.read(&mut bytes).expect("read kernel.elf");
    bytes.truncate(read);
    bytes
}

// ---------------------------------------------------------------------------
// Paging
// ---------------------------------------------------------------------------

fn alloc_pages(bt: &BootServices, n: usize) -> u64 {
    bt.allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, n)
        .expect("allocate_pages")
}

fn pages_for(bytes: u64) -> usize {
    ((bytes + 4095) / 4096) as usize
}

/// Build a fresh 4-level page table, returning the physical address of the
/// PML4 and the highest virtual address of the loaded kernel.
fn build_page_tables(bt: &BootServices, elf: &ElfFile, fb: &Framebuffer) -> (u64, u64) {
    let pml4_phys = alloc_pages(bt, 1);
    let pml4 = unsafe { &mut *(pml4_phys as *mut PageTable) };
    pml4.zero();

    // Identity map the first IDENTITY_MAP_BYTES with 2 MiB pages.
    map_region_2m(
        bt,
        pml4,
        0,
        0,
        IDENTITY_MAP_BYTES,
        PTF::PRESENT | PTF::WRITABLE,
    );

    // HHDM: map all of physical memory (at least the same region) at HHDM_OFFSET.
    map_region_2m(
        bt,
        pml4,
        HHDM_OFFSET,
        0,
        IDENTITY_MAP_BYTES,
        PTF::PRESENT | PTF::WRITABLE | PTF::NO_EXECUTE,
    );

    // Framebuffer: also map it through the HHDM explicitly (it may lie above 4 GiB).
    let fb_phys = fb.base as u64 - HHDM_OFFSET;
    let fb_bytes = (fb.pitch as u64) * (fb.height as u64);
    map_region_2m(
        bt,
        pml4,
        HHDM_OFFSET + (fb_phys & !0x1F_FFFF),
        fb_phys & !0x1F_FFFF,
        align_up(fb_bytes + (fb_phys & 0x1F_FFFF), 2 * 1024 * 1024),
        PTF::PRESENT | PTF::WRITABLE | PTF::NO_EXECUTE,
    );

    // Kernel PT_LOAD segments.
    let mut max_virt: u64 = 0;
    for ph in elf.program_iter() {
        if ph.get_type().ok() != Some(PhType::Load) {
            continue;
        }
        load_segment(bt, pml4, elf, &ph);
        let end = ph.virtual_addr() + ph.mem_size();
        if end > max_virt {
            max_virt = end;
        }
    }

    // Make sure paging features we rely on are enabled (PAE, NXE).
    unsafe {
        Cr4::update(|f| f.insert(Cr4Flags::PHYSICAL_ADDRESS_EXTENSION));
        use x86_64::registers::model_specific::{Efer, EferFlags};
        Efer::update(|f| f.insert(EferFlags::NO_EXECUTE_ENABLE));
        Cr0::update(|f| {
            f.insert(Cr0Flags::PAGING | Cr0Flags::PROTECTED_MODE_ENABLE | Cr0Flags::WRITE_PROTECT)
        });
    }

    (pml4_phys, max_virt)
}

fn load_segment(bt: &BootServices, pml4: &mut PageTable, elf: &ElfFile, ph: &ProgramHeader) {
    let virt = ph.virtual_addr();
    let file_size = ph.file_size();
    let mem_size = ph.mem_size();
    let file_off = ph.offset();

    let pages = pages_for(mem_size + (virt & 0xFFF));
    let seg_phys = alloc_pages(bt, pages);

    // Zero and copy file contents.
    unsafe {
        ptr::write_bytes(seg_phys as *mut u8, 0, pages * 4096);
        let src = elf.input.as_ptr().add(file_off as usize);
        let dst = (seg_phys + (virt & 0xFFF)) as *mut u8;
        ptr::copy_nonoverlapping(src, dst, file_size as usize);
    }

    let flags_src = ph.flags();
    let mut flags = PTF::PRESENT;
    if flags_src.is_write() {
        flags |= PTF::WRITABLE;
    }
    if !flags_src.is_execute() {
        flags |= PTF::NO_EXECUTE;
    }

    let virt_start = virt & !0xFFF;
    let length = pages as u64 * 4096;
    map_region_4k(bt, pml4, virt_start, seg_phys, length, flags);
}

fn align_up(x: u64, a: u64) -> u64 {
    (x + a - 1) & !(a - 1)
}

fn map_region_4k(
    bt: &BootServices,
    pml4: &mut PageTable,
    virt: u64,
    phys: u64,
    length: u64,
    flags: PTF,
) {
    let mut off = 0u64;
    while off < length {
        let v = VirtAddr::new(virt + off);
        let p = phys + off;
        let pt = walk_to_pt(bt, pml4, v);
        let idx = ((v.as_u64() >> 12) & 0x1FF) as usize;
        pt[idx].set_addr(PhysAddr::new(p), flags);
        off += 4096;
    }
}

fn map_region_2m(
    bt: &BootServices,
    pml4: &mut PageTable,
    virt: u64,
    phys: u64,
    length: u64,
    flags: PTF,
) {
    let mut off = 0u64;
    let huge = flags | PTF::HUGE_PAGE;
    while off < length {
        let v = VirtAddr::new(virt + off);
        let p = phys + off;
        let pd = walk_to_pd(bt, pml4, v);
        let idx = ((v.as_u64() >> 21) & 0x1FF) as usize;
        pd[idx].set_addr(PhysAddr::new(p), huge);
        off += 2 * 1024 * 1024;
    }
}

fn walk_to_pd<'a>(bt: &BootServices, pml4: &'a mut PageTable, v: VirtAddr) -> &'a mut PageTable {
    let pml4_idx = ((v.as_u64() >> 39) & 0x1FF) as usize;
    let pdpt = ensure_child(bt, &mut pml4[pml4_idx]);
    let pdpt_idx = ((v.as_u64() >> 30) & 0x1FF) as usize;
    ensure_child(bt, &mut pdpt[pdpt_idx])
}

fn walk_to_pt<'a>(bt: &BootServices, pml4: &'a mut PageTable, v: VirtAddr) -> &'a mut PageTable {
    let pd = walk_to_pd(bt, pml4, v);
    let pd_idx = ((v.as_u64() >> 21) & 0x1FF) as usize;
    ensure_child(bt, &mut pd[pd_idx])
}

fn ensure_child<'a>(
    bt: &BootServices,
    entry: &'a mut x86_64::structures::paging::page_table::PageTableEntry,
) -> &'a mut PageTable {
    if entry.is_unused() {
        let frame = alloc_pages(bt, 1);
        unsafe { ptr::write_bytes(frame as *mut u8, 0, 4096) };
        entry.set_addr(
            PhysAddr::new(frame),
            PTF::PRESENT | PTF::WRITABLE | PTF::USER_ACCESSIBLE,
        );
    }
    let child_phys = entry.addr().as_u64();
    unsafe { &mut *(child_phys as *mut PageTable) }
}

// ---------------------------------------------------------------------------
// ACPI RSDP
// ---------------------------------------------------------------------------

fn find_rsdp(st: &SystemTable<Boot>) -> u64 {
    let mut acpi1 = 0u64;
    for entry in st.config_table() {
        if entry.guid == ACPI2_GUID {
            return entry.address as u64;
        }
        if entry.guid == ACPI_GUID {
            acpi1 = entry.address as u64;
        }
    }
    acpi1
}
