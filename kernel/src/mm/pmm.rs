//! Physical frame allocator — single-bitmap, single-lock.
//!
//! We take ownership of every `Usable` or `BootloaderReclaimable` region the
//! bootloader handed us, allocate the tracking bitmap out of the first region
//! that is large enough, and then hand out 4 KiB frames.
//!
//! The allocator is intentionally boring: it serves M1's needs (small user
//! binaries, page tables, kernel heap) and is replaceable by a buddy once we
//! have measurements.

use core::sync::atomic::{AtomicUsize, Ordering};

use persona_shared::{BootInfo, MemoryKind, HHDM_OFFSET};
use spin::Mutex;

use super::PAGE_SIZE;

static PMM: Mutex<Option<Pmm>> = Mutex::new(None);
static TOTAL_FRAMES: AtomicUsize = AtomicUsize::new(0);
static FREE_FRAMES: AtomicUsize = AtomicUsize::new(0);

struct Pmm {
    bitmap: &'static mut [u8],
    /// Physical address of frame 0 (always 0 — bitmap is indexed by PFN).
    /// Kept explicit for future non-zero-origin setups.
    base_pfn: usize,
    total_pfns: usize,
    /// Hint for the next search — not a correctness field.
    next_hint: usize,
}

/// # Safety
/// Must be called exactly once during early boot, before the heap is online.
pub unsafe fn init(info: &BootInfo) {
    let regions =
        unsafe { core::slice::from_raw_parts(info.memory_map.entries, info.memory_map.count) };

    // Total PFNs = ceil(max_phys / PAGE_SIZE). Using the max extent rather
    // than just the usable sum lets us index the bitmap directly by PFN.
    let mut max_phys: u64 = 0;
    for r in regions {
        let end = r.base + r.length;
        if end > max_phys {
            max_phys = end;
        }
    }
    let total_pfns = max_phys.div_ceil(PAGE_SIZE as u64) as usize;
    let bitmap_bytes = total_pfns.div_ceil(8);
    let bitmap_pages = bitmap_bytes.div_ceil(PAGE_SIZE);

    // Carve bitmap out of the first usable region that fits.
    let mut bitmap_phys: Option<u64> = None;
    for r in regions {
        if !is_free(r.kind) {
            continue;
        }
        let pages = (r.length / PAGE_SIZE as u64) as usize;
        if pages >= bitmap_pages {
            bitmap_phys = Some(r.base);
            break;
        }
    }
    let bitmap_phys = bitmap_phys.expect("no usable region big enough for the PMM bitmap");

    // All frames start as reserved; we'll flip the free ones on below.
    let bitmap: &mut [u8] = unsafe {
        let ptr = (bitmap_phys + HHDM_OFFSET) as *mut u8;
        core::ptr::write_bytes(ptr, 0xFF, bitmap_bytes);
        core::slice::from_raw_parts_mut(ptr, bitmap_bytes)
    };

    let mut pmm = Pmm {
        bitmap,
        base_pfn: 0,
        total_pfns,
        next_hint: 0,
    };

    // Mark every free region's frames as available.
    let mut free = 0usize;
    for r in regions {
        if !is_free(r.kind) {
            continue;
        }
        let start_pfn = (r.base / PAGE_SIZE as u64) as usize;
        let pages = (r.length / PAGE_SIZE as u64) as usize;
        for i in 0..pages {
            let pfn = start_pfn + i;
            if pfn >= total_pfns {
                break;
            }
            pmm.clear(pfn);
            free += 1;
        }
    }

    // Reserve the bitmap's own pages and page 0 (never hand out NULL).
    for i in 0..bitmap_pages {
        let pfn = ((bitmap_phys / PAGE_SIZE as u64) as usize) + i;
        if !pmm.is_set(pfn) {
            pmm.set(pfn);
            free -= 1;
        }
    }
    if !pmm.is_set(0) {
        pmm.set(0);
        free = free.saturating_sub(1);
    }

    TOTAL_FRAMES.store(total_pfns, Ordering::Relaxed);
    FREE_FRAMES.store(free, Ordering::Relaxed);
    *PMM.lock() = Some(pmm);
}

fn is_free(kind: MemoryKind) -> bool {
    matches!(kind, MemoryKind::Usable | MemoryKind::BootloaderReclaimable)
}

impl Pmm {
    fn set(&mut self, pfn: usize) {
        self.bitmap[pfn / 8] |= 1 << (pfn % 8);
    }
    fn clear(&mut self, pfn: usize) {
        self.bitmap[pfn / 8] &= !(1 << (pfn % 8));
    }
    fn is_set(&self, pfn: usize) -> bool {
        (self.bitmap[pfn / 8] >> (pfn % 8)) & 1 == 1
    }

    fn alloc_one(&mut self) -> Option<usize> {
        let start = self.next_hint;
        let mut pfn = start;
        loop {
            if pfn >= self.total_pfns {
                pfn = 0;
            }
            if !self.is_set(pfn) {
                self.set(pfn);
                self.next_hint = pfn + 1;
                return Some(self.base_pfn + pfn);
            }
            pfn += 1;
            if pfn == start {
                return None;
            }
        }
    }

    #[allow(dead_code)]
    fn free_one(&mut self, pfn: usize) {
        self.clear(pfn);
        if pfn < self.next_hint {
            self.next_hint = pfn;
        }
    }
}

/// Allocate one 4 KiB physical frame. Returns the physical address.
pub fn alloc_frame() -> Option<u64> {
    let mut g = PMM.lock();
    let pfn = g.as_mut()?.alloc_one()?;
    FREE_FRAMES.fetch_sub(1, Ordering::Relaxed);
    Some((pfn * PAGE_SIZE) as u64)
}

/// Allocate `n` contiguous frames. Used for the kernel heap and for a few
/// early allocations that want physical contiguity. Linear scan — O(N·n).
pub fn alloc_contig(n: usize) -> Option<u64> {
    let mut g = PMM.lock();
    let pmm = g.as_mut()?;
    let mut run = 0usize;
    let mut start = 0usize;
    for pfn in 0..pmm.total_pfns {
        if pmm.is_set(pfn) {
            run = 0;
            continue;
        }
        if run == 0 {
            start = pfn;
        }
        run += 1;
        if run == n {
            for i in 0..n {
                pmm.set(start + i);
            }
            FREE_FRAMES.fetch_sub(n, Ordering::Relaxed);
            return Some((start * PAGE_SIZE) as u64);
        }
    }
    None
}

/// Free a frame previously returned by [`alloc_frame`] or [`alloc_contig`].
#[allow(dead_code)]
pub fn free_frame(phys: u64) {
    let pfn = (phys / PAGE_SIZE as u64) as usize;
    if let Some(pmm) = PMM.lock().as_mut() {
        pmm.free_one(pfn);
        FREE_FRAMES.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn total_frames() -> usize {
    TOTAL_FRAMES.load(Ordering::Relaxed)
}

pub fn free_frames() -> usize {
    FREE_FRAMES.load(Ordering::Relaxed)
}
