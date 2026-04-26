//! personaboot — UEFI bootloader for personaOS.
//!
//! Responsibilities:
//!   1. Obtain a linear framebuffer via GOP.
//!   2. Read `\EFI\personaOS\kernel.elf` from the ESP.
//!   3. Parse the ELF and allocate physical memory for each PT_LOAD segment.
//!   4. Build fresh page tables: identity map the low 4 GiB (for firmware),
//!      HHDM for all physical memory, and kernel segments at their
//!      linked addresses.
//!   5. Locate the ACPI RSDP in the UEFI configuration table.
//!   6. Construct a `BootInfo`.
//!   7. Call `exit_boot_services`, install CR3, switch to the kernel stack,
//!      and jump to the kernel.

#![no_main]
#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use core::arch::asm;
use core::ptr;

use persona_shared::{
    BootInfo, Framebuffer, MemoryKind, MemoryMap, MemoryRegion, PixelFormat, BOOT_INFO_MAGIC,
    BOOT_INFO_VERSION, HHDM_OFFSET,
};
use uefi::boot::{self, AllocateType, MemoryType, ScopedProtocol};
use uefi::mem::memory_map::MemoryMap as UefiMemoryMap;
use uefi::prelude::*;
use uefi::proto::console::gop::{GraphicsOutput, PixelFormat as GopPixelFormat};
use uefi::proto::media::file::{File, FileAttribute, FileInfo, FileMode, RegularFile};
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::system;
use uefi::table::cfg::ConfigTableEntry;
use uefi::CStr16;
use x86_64::registers::control::{Cr3, Cr3Flags};
use x86_64::structures::paging::{page_table::PageTableFlags as PTF, PageTable, PhysFrame};
use x86_64::{PhysAddr, VirtAddr};
use xmas_elf::program::{ProgramHeader, Type as PhType};
use xmas_elf::ElfFile;

const KERNEL_PATH: &str = "\\EFI\\personaOS\\kernel.elf";
const KERNEL_STACK_SIZE: usize = 64 * 1024;
/// Identity-map the first 4 GiB so firmware / MMIO remains reachable
/// immediately after switching to our page tables.
const IDENTITY_MAP_BYTES: u64 = 4 * 1024 * 1024 * 1024;

const MMAP_PAGES: usize = 16;

#[entry]
fn efi_main() -> Status {
    uefi::helpers::init().expect("uefi helpers init");
    log::info!("[personaboot] hello, world");

    let fb = init_framebuffer();
    log::info!(
        "[personaboot] framebuffer {}x{} pitch={} bpp={}",
        fb.width,
        fb.height,
        fb.pitch,
        fb.bits_per_pixel
    );

    let kernel_bytes = load_kernel();
    log::info!(
        "[personaboot] loaded kernel.elf: {} bytes",
        kernel_bytes.len()
    );

    let elf = ElfFile::new(&kernel_bytes).expect("parse kernel ELF");
    let entry_point = elf.header.pt2.entry_point();
    log::info!("[personaboot] kernel entry = {:#x}", entry_point);

    let stack_phys = alloc_pages(pages_for(KERNEL_STACK_SIZE as u64));
    let stack_top_virt = HHDM_OFFSET + stack_phys + KERNEL_STACK_SIZE as u64;
    log::info!("[personaboot] kernel stack top = {:#x}", stack_top_virt);

    let rsdp = find_rsdp();
    log::info!("[personaboot] rsdp = {:#x}", rsdp);

    let (pml4_phys, _loaded) = build_page_tables(&elf, &fb);
    log::info!("[personaboot] pml4 @ {:#x}", pml4_phys);

    // Allocate BootInfo + memory-map storage. These are written through the
    // identity map while firmware's page tables are active; the kernel will
    // read them through the HHDM after CR3 is installed.
    let boot_info_phys = alloc_pages(1);
    let mmap_entries_phys = alloc_pages(MMAP_PAGES);
    let mmap_capacity = (MMAP_PAGES * 4096) / core::mem::size_of::<MemoryRegion>();

    // From here on, no more prints or allocations.
    let mmap = unsafe { boot::exit_boot_services(None) };

    let mmap_ptr = mmap_entries_phys as *mut MemoryRegion;
    let mut count = 0usize;
    for desc in UefiMemoryMap::entries(&mmap) {
        if count >= mmap_capacity {
            break;
        }
        let kind = match desc.ty {
            MemoryType::CONVENTIONAL => MemoryKind::Usable,
            MemoryType::BOOT_SERVICES_CODE | MemoryType::BOOT_SERVICES_DATA => {
                MemoryKind::BootloaderReclaimable
            }
            MemoryType::LOADER_CODE | MemoryType::LOADER_DATA => {
                MemoryKind::BootloaderReclaimable
            }
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

    let mmap_virt = (mmap_entries_phys + HHDM_OFFSET) as *const MemoryRegion;
    let info = BootInfo {
        magic: BOOT_INFO_MAGIC,
        version: BOOT_INFO_VERSION,
        _pad0: 0,
        framebuffer: fb,
        memory_map: MemoryMap {
            entries: mmap_virt,
            count,
        },
        rsdp_phys: rsdp,
        cmdline_ptr: ptr::null(),
        cmdline_len: 0,
        hhdm_offset: HHDM_OFFSET,
    };
    unsafe { ptr::write(boot_info_phys as *mut BootInfo, info) };
    let info_virt = (boot_info_phys + HHDM_OFFSET) as *const BootInfo;

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
            in("rdi") info_virt as u64,
            options(noreturn)
        );
    }
}

// ---------------------------------------------------------------------------
// Framebuffer
// ---------------------------------------------------------------------------

fn init_framebuffer() -> Framebuffer {
    let handle = boot::get_handle_for_protocol::<GraphicsOutput>().expect("locate GOP handle");
    let mut gop: ScopedProtocol<GraphicsOutput> =
        boot::open_protocol_exclusive::<GraphicsOutput>(handle).expect("open GOP");

    let mut best: Option<(usize, (u32, u32), GopPixelFormat)> = None;
    for (idx, mode) in gop.modes().enumerate() {
        let info = mode.info();
        let (w, h) = info.resolution();
        let pf = info.pixel_format();
        if !matches!(pf, GopPixelFormat::Rgb | GopPixelFormat::Bgr) {
            continue;
        }
        let px = w as u64 * h as u64;
        if best.is_none_or(|(_, (bw, bh), _)| (bw as u64 * bh as u64) < px) {
            best = Some((idx, (w as u32, h as u32), pf));
        }
    }
    if let Some((idx, _, _)) = best {
        let mode = gop.modes().nth(idx).unwrap();
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

fn load_kernel() -> Vec<u8> {
    let image = boot::image_handle();
    let mut sfs: ScopedProtocol<SimpleFileSystem> =
        boot::get_image_file_system(image).expect("open image filesystem");
    let mut root = sfs.open_volume().expect("open ESP root");

    let mut buf = [0u16; 64];
    let path = CStr16::from_str_with_buf(KERNEL_PATH, &mut buf).expect("kernel path -> CStr16");

    let file = root
        .open(path, FileMode::Read, FileAttribute::empty())
        .expect("open kernel.elf");
    let mut file: RegularFile = file.into_regular_file().expect("kernel.elf is a regular file");

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

fn alloc_pages(n: usize) -> u64 {
    boot::allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, n)
        .expect("allocate_pages")
        .as_ptr() as u64
}

fn pages_for(bytes: u64) -> usize {
    bytes.div_ceil(4096) as usize
}

fn build_page_tables(elf: &ElfFile, fb: &Framebuffer) -> (u64, u64) {
    let pml4_phys = alloc_pages(1);
    let pml4 = unsafe { &mut *(pml4_phys as *mut PageTable) };
    pml4.zero();

    map_region_2m(
        pml4,
        0,
        0,
        IDENTITY_MAP_BYTES,
        PTF::PRESENT | PTF::WRITABLE,
    );

    map_region_2m(
        pml4,
        HHDM_OFFSET,
        0,
        IDENTITY_MAP_BYTES,
        PTF::PRESENT | PTF::WRITABLE | PTF::NO_EXECUTE,
    );

    let fb_phys = fb.base as u64 - HHDM_OFFSET;
    let fb_bytes = (fb.pitch as u64) * (fb.height as u64);
    map_region_2m(
        pml4,
        HHDM_OFFSET + (fb_phys & !0x1F_FFFF),
        fb_phys & !0x1F_FFFF,
        align_up(fb_bytes + (fb_phys & 0x1F_FFFF), 2 * 1024 * 1024),
        PTF::PRESENT | PTF::WRITABLE | PTF::NO_EXECUTE,
    );

    let mut max_virt: u64 = 0;
    for ph in elf.program_iter() {
        if ph.get_type().ok() != Some(PhType::Load) {
            continue;
        }
        load_segment(pml4, elf, &ph);
        let end = ph.virtual_addr() + ph.mem_size();
        if end > max_virt {
            max_virt = end;
        }
    }

    unsafe {
        use x86_64::registers::model_specific::{Efer, EferFlags};
        Efer::update(|f| f.insert(EferFlags::NO_EXECUTE_ENABLE));
    }

    (pml4_phys, max_virt)
}

fn load_segment(pml4: &mut PageTable, elf: &ElfFile, ph: &ProgramHeader) {
    let virt = ph.virtual_addr();
    let file_size = ph.file_size();
    let mem_size = ph.mem_size();
    let file_off = ph.offset();

    let pages = pages_for(mem_size + (virt & 0xFFF));
    let seg_phys = alloc_pages(pages);

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
    map_region_4k(pml4, virt_start, seg_phys, length, flags);
}

fn align_up(x: u64, a: u64) -> u64 {
    (x + a - 1) & !(a - 1)
}

fn map_region_4k(pml4: &mut PageTable, virt: u64, phys: u64, length: u64, flags: PTF) {
    let mut off = 0u64;
    while off < length {
        let v = VirtAddr::new(virt + off);
        let p = phys + off;
        let pt = walk_to_pt(pml4, v);
        let idx = ((v.as_u64() >> 12) & 0x1FF) as usize;
        pt[idx].set_addr(PhysAddr::new(p), flags);
        off += 4096;
    }
}

fn map_region_2m(pml4: &mut PageTable, virt: u64, phys: u64, length: u64, flags: PTF) {
    let mut off = 0u64;
    let huge = flags | PTF::HUGE_PAGE;
    while off < length {
        let v = VirtAddr::new(virt + off);
        let p = phys + off;
        let pd = walk_to_pd(pml4, v);
        let idx = ((v.as_u64() >> 21) & 0x1FF) as usize;
        pd[idx].set_addr(PhysAddr::new(p), huge);
        off += 2 * 1024 * 1024;
    }
}

fn walk_to_pd(pml4: &mut PageTable, v: VirtAddr) -> &mut PageTable {
    let pml4_idx = ((v.as_u64() >> 39) & 0x1FF) as usize;
    let pdpt = ensure_child(&mut pml4[pml4_idx]);
    let pdpt_idx = ((v.as_u64() >> 30) & 0x1FF) as usize;
    ensure_child(&mut pdpt[pdpt_idx])
}

fn walk_to_pt(pml4: &mut PageTable, v: VirtAddr) -> &mut PageTable {
    let pd = walk_to_pd(pml4, v);
    let pd_idx = ((v.as_u64() >> 21) & 0x1FF) as usize;
    ensure_child(&mut pd[pd_idx])
}

fn ensure_child(
    entry: &mut x86_64::structures::paging::page_table::PageTableEntry,
) -> &mut PageTable {
    if entry.is_unused() {
        let frame = alloc_pages(1);
        unsafe { ptr::write_bytes(frame as *mut u8, 0, 4096) };
        entry.set_addr(PhysAddr::new(frame), PTF::PRESENT | PTF::WRITABLE);
    }
    let child_phys = entry.addr().as_u64();
    unsafe { &mut *(child_phys as *mut PageTable) }
}

// ---------------------------------------------------------------------------
// ACPI RSDP
// ---------------------------------------------------------------------------

fn find_rsdp() -> u64 {
    let mut acpi1 = 0u64;
    let acpi2 = system::with_config_table(|slice| {
        for entry in slice {
            if entry.guid == ConfigTableEntry::ACPI2_GUID {
                return entry.address as u64;
            }
            if entry.guid == ConfigTableEntry::ACPI_GUID {
                acpi1 = entry.address as u64;
            }
        }
        0
    });
    if acpi2 != 0 {
        acpi2
    } else {
        acpi1
    }
}

