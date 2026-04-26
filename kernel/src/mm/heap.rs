//! Kernel heap — a 4 MiB linked-list allocator sitting in a contiguous
//! physical region we reserve up front. Good enough for M1; replaceable with
//! a slab once profiling says so.

use linked_list_allocator::LockedHeap;
use persona_shared::HHDM_OFFSET;

use super::{pmm, PAGE_SIZE};

pub const HEAP_SIZE: usize = 4 * 1024 * 1024;

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// # Safety
/// Call once, after the PMM is live.
pub unsafe fn init() {
    let pages = HEAP_SIZE / PAGE_SIZE;
    let phys = pmm::alloc_contig(pages).expect("heap: no contiguous physical region");
    let virt = (phys + HHDM_OFFSET) as *mut u8;
    unsafe {
        ALLOCATOR.lock().init(virt, HEAP_SIZE);
    }
}
