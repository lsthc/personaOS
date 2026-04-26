//! Boot protocol between `personaboot` and the kernel.
//!
//! The bootloader constructs a [`BootInfo`] in memory it has identity-mapped
//! and passes a pointer to it via `rdi` when jumping into the kernel. All
//! pointers inside refer to memory the bootloader has mapped into the
//! higher-half direct map (HHDM). The kernel takes ownership of these
//! regions on entry; the bootloader does not run again.

/// ASCII "PondOSB\0" as a little-endian u64 (`P` is the lowest-addressed byte).
pub const BOOT_INFO_MAGIC: u64 = u64::from_le_bytes(*b"PondOSB\0");
pub const BOOT_INFO_VERSION: u32 = 1;

/// Offset at which the bootloader maps all physical memory in the kernel's
/// address space. `phys + HHDM_OFFSET == virt` for any physical address.
pub const HHDM_OFFSET: u64 = 0xFFFF_8000_0000_0000;

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct BootInfo {
    pub magic: u64,
    pub version: u32,
    pub _pad0: u32,

    pub framebuffer: Framebuffer,
    pub memory_map: MemoryMap,

    /// Physical address of the ACPI RSDP (Root System Description Pointer).
    /// Zero if not present.
    pub rsdp_phys: u64,

    /// Virtual address of a UTF-8 kernel command line (null terminated).
    /// May be null.
    pub cmdline_ptr: *const u8,
    pub cmdline_len: usize,

    /// The higher-half offset where all of physical memory is mapped.
    pub hhdm_offset: u64,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Framebuffer {
    /// Virtual (HHDM) address of the linear framebuffer.
    pub base: *mut u8,
    pub width: u32,
    pub height: u32,
    /// Bytes between the start of one scanline and the next.
    pub pitch: u32,
    pub bits_per_pixel: u32,
    pub pixel_format: PixelFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum PixelFormat {
    /// Pixel is 32 bits: B, G, R, reserved (little-endian byte order).
    Bgrx8888 = 0,
    /// Pixel is 32 bits: R, G, B, reserved.
    Rgbx8888 = 1,
    Unknown = 0xFFFF_FFFF,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct MemoryMap {
    /// Virtual (HHDM) address of an array of [`MemoryRegion`].
    pub entries: *const MemoryRegion,
    pub count: usize,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct MemoryRegion {
    pub base: u64,
    pub length: u64,
    pub kind: MemoryKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum MemoryKind {
    /// Conventional RAM, free for the kernel's physical frame allocator.
    Usable = 0,
    /// Contains UEFI boot-services / bootloader data that the kernel may
    /// reclaim once it no longer needs the handoff structures.
    BootloaderReclaimable = 1,
    /// Kernel image (code + data). Must not be reclaimed.
    KernelAndModules = 2,
    /// ACPI tables the kernel may reclaim after parsing.
    AcpiReclaimable = 3,
    /// ACPI non-volatile storage. Keep mapped.
    AcpiNvs = 4,
    /// Reserved / memory-mapped devices. Do not touch.
    Reserved = 5,
    BadMemory = 6,
    /// The framebuffer region.
    Framebuffer = 7,
}

// BootInfo is passed by pointer across a defined ABI, but never shared between
// threads before the kernel is ready. We mark it Send/Sync to let the kernel
// wrap it in synchronization primitives as needed.
unsafe impl Send for BootInfo {}
unsafe impl Sync for BootInfo {}
