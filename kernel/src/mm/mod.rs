//! Memory management — physical allocator, virtual mapping, kernel heap.

pub mod heap;
pub mod pmm;
pub mod vmm;

pub const PAGE_SIZE: usize = 4096;

/// Page-align `x` upwards.
#[allow(dead_code)]
pub const fn page_up(x: usize) -> usize {
    (x + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}

/// Bring up physical + heap allocators, then the VMM. Must run before any
/// heap-using code.
///
/// # Safety
/// Caller must pass the bootloader's memory map pointer/len exactly once,
/// and must not hold any stale pointers into `BootloaderReclaimable` memory.
pub unsafe fn init(info: &persona_shared::BootInfo) {
    unsafe {
        pmm::init(info);
        heap::init();
        vmm::init(info);
    }
}
