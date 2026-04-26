//! Virtual memory manager — walks the active PML4 via the HHDM to map and
//! unmap 4 KiB pages. The PML4 physical address comes from CR3 on
//! initialization so we inherit whatever the bootloader built.

use bitflags::bitflags;
use persona_shared::{BootInfo, HHDM_OFFSET};
use spin::Mutex;
use x86_64::registers::control::Cr3;
use x86_64::structures::paging::{page_table::PageTableFlags as PTF, PageTable};
use x86_64::PhysAddr;

use super::{pmm, PAGE_SIZE};

bitflags! {
    #[derive(Clone, Copy)]
    pub struct MapFlags: u64 {
        const WRITE = 1 << 0;
        const USER  = 1 << 1;
        const NX    = 1 << 2;
        /// Uncached (PCD | PWT). Used for device MMIO so stores aren't held
        /// in the write buffer or cached on a later read.
        const MMIO  = 1 << 3;
    }
}

impl MapFlags {
    fn leaf(self) -> PTF {
        let mut f = PTF::PRESENT;
        if self.contains(Self::WRITE) {
            f |= PTF::WRITABLE;
        }
        if self.contains(Self::USER) {
            f |= PTF::USER_ACCESSIBLE;
        }
        if self.contains(Self::NX) {
            f |= PTF::NO_EXECUTE;
        }
        if self.contains(Self::MMIO) {
            f |= PTF::NO_CACHE | PTF::WRITE_THROUGH;
        }
        f
    }

    fn intermediate(self) -> PTF {
        // Intermediate entries grant the union of possible rights; the leaf
        // decides what sticks. We always set WRITABLE and, if any child is
        // userspace, USER_ACCESSIBLE.
        let mut f = PTF::PRESENT | PTF::WRITABLE;
        if self.contains(Self::USER) {
            f |= PTF::USER_ACCESSIBLE;
        }
        f
    }
}

pub struct AddressSpace {
    pml4_phys: u64,
}

static KERNEL_AS: Mutex<Option<AddressSpace>> = Mutex::new(None);

// Bump pointer for kernel MMIO mappings. We keep MMIO out of the HHDM so we
// can pin the cacheability to UC without fighting the HHDM's WB mapping of
// the same physical page. The MMIO window lives above the HHDM and below the
// kernel image. Consumed by M2 drivers (pci/nvme/xhci).
#[allow(dead_code)]
const MMIO_VA_BASE: u64 = 0xFFFF_9000_0000_0000;
#[allow(dead_code)]
const MMIO_VA_END: u64 = 0xFFFF_9000_4000_0000; // 1 GiB is plenty for M2.
#[allow(dead_code)]
static MMIO_NEXT: Mutex<u64> = Mutex::new(MMIO_VA_BASE);

/// # Safety
/// Call exactly once, after the PMM is up. Reads CR3 to inherit the
/// bootloader's PML4.
pub unsafe fn init(_info: &BootInfo) {
    let (frame, _flags) = Cr3::read();
    *KERNEL_AS.lock() = Some(AddressSpace {
        pml4_phys: frame.start_address().as_u64(),
    });
}

#[allow(dead_code)]
pub fn kernel() -> AddressSpace {
    AddressSpace {
        pml4_phys: KERNEL_AS.lock().as_ref().unwrap().pml4_phys,
    }
}

impl AddressSpace {
    #[allow(dead_code)]
    pub fn pml4_phys(&self) -> u64 {
        self.pml4_phys
    }

    /// Create a fresh address space that shares the upper-half (kernel)
    /// mappings with the kernel PML4. Used when spawning a user process.
    pub fn new_user() -> Option<Self> {
        let phys = pmm::alloc_frame()?;
        unsafe {
            let new_pml4 = &mut *((phys + HHDM_OFFSET) as *mut PageTable);
            new_pml4.zero();
            let k_phys = KERNEL_AS.lock().as_ref().unwrap().pml4_phys;
            let kernel_pml4 = &*((k_phys + HHDM_OFFSET) as *const PageTable);
            // PML4 indices 256..512 cover the higher half.
            for i in 256..512 {
                new_pml4[i] = kernel_pml4[i].clone();
            }
        }
        Some(Self { pml4_phys: phys })
    }

    /// Switch CR3 to this address space. Caller is responsible for flushing
    /// user mappings if the previous AS was a sibling.
    ///
    /// # Safety
    /// Must be invoked with interrupts disabled during a task switch, and the
    /// address space must contain a valid copy of the kernel's higher half.
    pub unsafe fn activate(&self) {
        use x86_64::registers::control::{Cr3Flags, Cr3 as Cr3Reg};
        use x86_64::structures::paging::PhysFrame;
        unsafe {
            Cr3Reg::write(
                PhysFrame::from_start_address(PhysAddr::new(self.pml4_phys)).unwrap(),
                Cr3Flags::empty(),
            );
        }
    }

    /// Map `virt` → `phys` with `flags`. `virt` and `phys` must be
    /// page-aligned.
    ///
    /// # Safety
    /// Caller must ensure the mapping doesn't conflict with existing
    /// obligations (e.g. aliasing a kernel page as user).
    pub unsafe fn map_4k(&self, virt: u64, phys: u64, flags: MapFlags) -> Result<(), MapError> {
        if virt & 0xFFF != 0 || phys & 0xFFF != 0 {
            return Err(MapError::Misaligned);
        }
        let pml4 = pml4_ref(self.pml4_phys);
        let pdpt = ensure_child(&mut pml4[pml4_idx(virt)], flags.intermediate())?;
        let pd = ensure_child(&mut pdpt[pdpt_idx(virt)], flags.intermediate())?;
        let pt = ensure_child(&mut pd[pd_idx(virt)], flags.intermediate())?;
        let slot = &mut pt[pt_idx(virt)];
        if slot.flags().contains(PTF::PRESENT) {
            return Err(MapError::AlreadyMapped);
        }
        slot.set_addr(PhysAddr::new(phys), flags.leaf());
        Ok(())
    }

    /// Allocate `pages` fresh physical frames and map them starting at
    /// `virt`. Returns on the first failure with the partial mapping left in
    /// place — M1 is happy to leak on OOM.
    pub fn map_anon(&self, virt: u64, pages: usize, flags: MapFlags) -> Result<(), MapError> {
        for i in 0..pages {
            let frame = pmm::alloc_frame().ok_or(MapError::OutOfMemory)?;
            // Zero the freshly allocated frame through the HHDM.
            unsafe {
                core::ptr::write_bytes((frame + HHDM_OFFSET) as *mut u8, 0, PAGE_SIZE);
                self.map_4k(virt + (i * PAGE_SIZE) as u64, frame, flags)?;
            }
        }
        Ok(())
    }

    /// Unmap one 4 KiB page. Does not free the underlying frame.
    ///
    /// # Safety
    /// The virtual range must not be actively referenced.
    #[allow(dead_code)]
    pub unsafe fn unmap_4k(&self, virt: u64) -> Option<u64> {
        let pml4 = pml4_ref(self.pml4_phys);
        let pdpt = unsafe { child_mut(&mut pml4[pml4_idx(virt)])? };
        let pd = unsafe { child_mut(&mut pdpt[pdpt_idx(virt)])? };
        let pt = unsafe { child_mut(&mut pd[pd_idx(virt)])? };
        let slot = &mut pt[pt_idx(virt)];
        if !slot.flags().contains(PTF::PRESENT) {
            return None;
        }
        let phys = slot.addr().as_u64();
        slot.set_unused();
        x86_64::instructions::tlb::flush(x86_64::VirtAddr::new(virt));
        Some(phys)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapError {
    OutOfMemory,
    AlreadyMapped,
    Misaligned,
    #[allow(dead_code)] // returned by map_mmio once MMIO window fills
    NoVirtualSpace,
}

#[allow(dead_code)] // consumed by M2 drivers
/// Map a physical MMIO window (e.g. a PCI BAR, LAPIC, xHCI regs) into the
/// kernel address space as uncached memory and return the virtual address
/// corresponding to `phys`.
///
/// `bytes` is the window size; it is rounded up to a page. Mappings are
/// non-executable, writable, and kernel-only. The allocation is permanent
/// for the life of the kernel — there is no `unmap_mmio` yet.
pub fn map_mmio(phys: u64, bytes: usize) -> Result<u64, MapError> {
    let page_off = phys & (PAGE_SIZE as u64 - 1);
    let first = phys & !(PAGE_SIZE as u64 - 1);
    let last = (phys + bytes as u64 + PAGE_SIZE as u64 - 1) & !(PAGE_SIZE as u64 - 1);
    let pages = ((last - first) / PAGE_SIZE as u64) as usize;

    let virt_base = {
        let mut next = MMIO_NEXT.lock();
        let base = *next;
        let end = base + (pages * PAGE_SIZE) as u64;
        if end > MMIO_VA_END {
            return Err(MapError::NoVirtualSpace);
        }
        *next = end;
        base
    };

    let kernel = KERNEL_AS.lock();
    let as_ = kernel.as_ref().expect("VMM not initialized");
    for i in 0..pages {
        let v = virt_base + (i * PAGE_SIZE) as u64;
        let p = first + (i * PAGE_SIZE) as u64;
        unsafe {
            as_.map_4k(v, p, MapFlags::WRITE | MapFlags::NX | MapFlags::MMIO)?;
        }
    }
    Ok(virt_base + page_off)
}

// ---------------------------------------------------------------------------

fn pml4_ref(phys: u64) -> &'static mut PageTable {
    unsafe { &mut *((phys + HHDM_OFFSET) as *mut PageTable) }
}

#[allow(dead_code)]
fn child_ref(entry: &x86_64::structures::paging::page_table::PageTableEntry) -> Option<&'static PageTable> {
    if !entry.flags().contains(PTF::PRESENT) {
        return None;
    }
    let phys = entry.addr().as_u64();
    unsafe { Some(&*((phys + HHDM_OFFSET) as *const PageTable)) }
}

/// # Safety
/// Caller must ensure the PML4 chain is consistent and not concurrently
/// mutated through another reference.
#[allow(dead_code)]
unsafe fn child_mut(
    entry: &mut x86_64::structures::paging::page_table::PageTableEntry,
) -> Option<&'static mut PageTable> {
    if !entry.flags().contains(PTF::PRESENT) {
        return None;
    }
    let phys = entry.addr().as_u64();
    unsafe { Some(&mut *((phys + HHDM_OFFSET) as *mut PageTable)) }
}

fn ensure_child(
    entry: &mut x86_64::structures::paging::page_table::PageTableEntry,
    inter_flags: PTF,
) -> Result<&'static mut PageTable, MapError> {
    let phys = if entry.flags().contains(PTF::PRESENT) {
        // Widen flags if the existing entry is more restrictive than the
        // request (e.g. kernel-only parent now needs to host a user leaf).
        let existing = entry.flags();
        let union = existing | inter_flags;
        if union != existing {
            entry.set_flags(union);
        }
        entry.addr().as_u64()
    } else {
        let new = pmm::alloc_frame().ok_or(MapError::OutOfMemory)?;
        unsafe {
            core::ptr::write_bytes((new + HHDM_OFFSET) as *mut u8, 0, PAGE_SIZE);
        }
        entry.set_addr(PhysAddr::new(new), inter_flags);
        new
    };
    Ok(unsafe { &mut *((phys + HHDM_OFFSET) as *mut PageTable) })
}

fn pml4_idx(v: u64) -> usize { ((v >> 39) & 0x1FF) as usize }
fn pdpt_idx(v: u64) -> usize { ((v >> 30) & 0x1FF) as usize }
fn pd_idx(v: u64) -> usize { ((v >> 21) & 0x1FF) as usize }
fn pt_idx(v: u64) -> usize { ((v >> 12) & 0x1FF) as usize }
