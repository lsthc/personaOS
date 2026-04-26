//! Minimal NVMe 1.x driver.
//!
//! Supports one controller, one namespace, 4 KiB-or-smaller PRP1-only
//! transfers. Admin/IO queue pairs are 64 entries each. Completions are
//! polled — no MSI-X yet (the scheduler's wait-queue infra is ready when
//! we want to flip the switch).

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt::Write as _;
use core::sync::atomic::{fence, Ordering};

use spin::Mutex;

use crate::drivers::block::{BlockDevice, BlockError};
use crate::drivers::serial::SerialPort;
use crate::mm::pmm;
use crate::mm::vmm;
use crate::mm::PAGE_SIZE;
use persona_shared::HHDM_OFFSET;

// -- Controller Properties (BAR0) offsets -----------------------------------
const REG_CAP: usize = 0x00; // 8
const REG_CC: usize = 0x14; // 4
const REG_CSTS: usize = 0x1C; // 4
const REG_AQA: usize = 0x24; // 4
const REG_ASQ: usize = 0x28; // 8
const REG_ACQ: usize = 0x30; // 8

// Controller configuration (CC) bits.
const CC_EN: u32 = 1;
const CC_IOSQES_6: u32 = 6 << 16;
const CC_IOCQES_4: u32 = 4 << 20;

const CSTS_RDY: u32 = 1;

const ADMIN_Q_DEPTH: u32 = 64;
const IO_Q_DEPTH: u32 = 64;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct SubmissionEntry {
    // DWORD 0: opcode + fuse + psdt + cid
    cdw0: u32,
    nsid: u32,
    _rsvd: [u32; 2],
    mptr: u64,
    prp1: u64,
    prp2: u64,
    cdw10: u32,
    cdw11: u32,
    cdw12: u32,
    cdw13: u32,
    cdw14: u32,
    cdw15: u32,
}

impl SubmissionEntry {
    const fn zero() -> Self {
        Self {
            cdw0: 0,
            nsid: 0,
            _rsvd: [0; 2],
            mptr: 0,
            prp1: 0,
            prp2: 0,
            cdw10: 0,
            cdw11: 0,
            cdw12: 0,
            cdw13: 0,
            cdw14: 0,
            cdw15: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CompletionEntry {
    cdw0: u32,
    _rsvd: u32,
    sq_head: u16,
    sq_id: u16,
    cid: u16,
    status: u16,
}

struct Queue {
    sq_phys: u64,
    cq_phys: u64,
    sq_depth: u32,
    cq_depth: u32,
    sq_tail: u32,
    cq_head: u32,
    phase: u16,
    doorbell_sq: *mut u32,
    doorbell_cq: *mut u32,
    next_cid: u16,
}

// SAFETY: queue access is serialized by the containing Mutex<Controller>.
unsafe impl Send for Queue {}
unsafe impl Sync for Queue {}

impl Queue {
    fn sq_slot(&mut self, i: u32) -> &mut SubmissionEntry {
        let ptr = (self.sq_phys + HHDM_OFFSET) as *mut SubmissionEntry;
        unsafe { &mut *ptr.add(i as usize) }
    }
    fn cq_slot(&self, i: u32) -> &CompletionEntry {
        let ptr = (self.cq_phys + HHDM_OFFSET) as *const CompletionEntry;
        unsafe { &*ptr.add(i as usize) }
    }

    /// Submit one command, poll for completion, return (cdw0, status).
    fn submit_sync(&mut self, mut cmd: SubmissionEntry) -> (u32, u16) {
        let cid = self.next_cid;
        self.next_cid = self.next_cid.wrapping_add(1);
        cmd.cdw0 = (cmd.cdw0 & 0x0000_FFFF) | ((cid as u32) << 16);

        let slot = self.sq_tail;
        *self.sq_slot(slot) = cmd;
        self.sq_tail = (self.sq_tail + 1) % self.sq_depth;

        // Ring the SQ doorbell.
        fence(Ordering::SeqCst);
        unsafe { self.doorbell_sq.write_volatile(self.sq_tail) };

        // Poll the CQ head entry until its phase flips.
        loop {
            let cqe = self.cq_slot(self.cq_head);
            let phase = cqe.status & 1;
            if phase == self.phase {
                let status = cqe.status >> 1;
                let cdw0 = cqe.cdw0;
                self.cq_head = (self.cq_head + 1) % self.cq_depth;
                if self.cq_head == 0 {
                    self.phase ^= 1;
                }
                fence(Ordering::SeqCst);
                unsafe { self.doorbell_cq.write_volatile(self.cq_head) };
                return (cdw0, status);
            }
            core::hint::spin_loop();
        }
    }
}

pub struct Controller {
    mmio: u64, // virt address of BAR0
    admin: Queue,
    io: Option<Queue>,
    doorbell_stride: u32,
}

impl Controller {
    fn read32(&self, off: usize) -> u32 {
        unsafe { ((self.mmio + off as u64) as *const u32).read_volatile() }
    }
    fn write32(&self, off: usize, v: u32) {
        unsafe { ((self.mmio + off as u64) as *mut u32).write_volatile(v) }
    }
    fn read64(&self, off: usize) -> u64 {
        unsafe { ((self.mmio + off as u64) as *const u64).read_volatile() }
    }
    fn write64(&self, off: usize, v: u64) {
        unsafe { ((self.mmio + off as u64) as *mut u64).write_volatile(v) }
    }

    fn doorbell(&self, qid: u32, completion: bool) -> *mut u32 {
        let idx = qid * 2 + if completion { 1 } else { 0 };
        let off = 0x1000u64 + idx as u64 * ((1 << self.doorbell_stride) * 4) as u64;
        (self.mmio + off) as *mut u32
    }
}

#[allow(dead_code)] // fields read by BlockDevice impl below
struct NvmeNs {
    controller: Arc<Mutex<Controller>>,
    nsid: u32,
    lba_bytes: u32,
    block_count: u64,
}

impl BlockDevice for NvmeNs {
    fn block_size(&self) -> usize { self.lba_bytes as usize }
    fn block_count(&self) -> u64 { self.block_count }

    fn read_blocks(&self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        if !buf.len().is_multiple_of(self.lba_bytes as usize) {
            return Err(BlockError::BadLength);
        }
        let nblocks = (buf.len() / self.lba_bytes as usize) as u32;
        if nblocks == 0 {
            return Ok(());
        }
        if lba.saturating_add(nblocks as u64) > self.block_count {
            return Err(BlockError::OutOfRange);
        }
        // PRP1-only: must fit inside a single 4 KiB page.
        if buf.len() > PAGE_SIZE {
            return Err(BlockError::Unsupported);
        }
        let frame = pmm::alloc_frame().ok_or(BlockError::Io)?;
        let result = (|| {
            let mut cmd = SubmissionEntry::zero();
            cmd.cdw0 = 0x02; // Read
            cmd.nsid = self.nsid;
            cmd.prp1 = frame;
            cmd.cdw10 = lba as u32;
            cmd.cdw11 = (lba >> 32) as u32;
            cmd.cdw12 = nblocks - 1;
            let mut g = self.controller.lock();
            let io = g.io.as_mut().ok_or(BlockError::Io)?;
            let (_, status) = io.submit_sync(cmd);
            if status != 0 {
                return Err(BlockError::Io);
            }
            let src = (frame + HHDM_OFFSET) as *const u8;
            unsafe {
                core::ptr::copy_nonoverlapping(src, buf.as_mut_ptr(), buf.len());
            }
            Ok(())
        })();
        pmm::free_frame(frame);
        result
    }

    fn write_blocks(&self, lba: u64, buf: &[u8]) -> Result<(), BlockError> {
        if !buf.len().is_multiple_of(self.lba_bytes as usize) {
            return Err(BlockError::BadLength);
        }
        let nblocks = (buf.len() / self.lba_bytes as usize) as u32;
        if nblocks == 0 {
            return Ok(());
        }
        if lba.saturating_add(nblocks as u64) > self.block_count {
            return Err(BlockError::OutOfRange);
        }
        if buf.len() > PAGE_SIZE {
            return Err(BlockError::Unsupported);
        }
        let frame = pmm::alloc_frame().ok_or(BlockError::Io)?;
        let dst = (frame + HHDM_OFFSET) as *mut u8;
        unsafe {
            core::ptr::copy_nonoverlapping(buf.as_ptr(), dst, buf.len());
        }
        let mut cmd = SubmissionEntry::zero();
        cmd.cdw0 = 0x01; // Write
        cmd.nsid = self.nsid;
        cmd.prp1 = frame;
        cmd.cdw10 = lba as u32;
        cmd.cdw11 = (lba >> 32) as u32;
        cmd.cdw12 = nblocks - 1;
        let result = {
            let mut g = self.controller.lock();
            let io = g.io.as_mut().ok_or(BlockError::Io)?;
            let (_, status) = io.submit_sync(cmd);
            if status != 0 { Err(BlockError::Io) } else { Ok(()) }
        };
        pmm::free_frame(frame);
        result
    }
}

/// Probe the PCI bus for an NVMe controller and bring it up. Returns the
/// registered block device on success.
pub fn init_from_pci() -> Option<Arc<dyn BlockDevice>> {
    let mut serial = unsafe { SerialPort::new(0x3F8) };
    let dev = crate::drivers::pci::find_class(0x01, 0x08, 0x02)?;
    let bar0 = dev.bars[0]?;
    if bar0.io || bar0.size < 0x2000 {
        return None;
    }
    let _ = writeln!(
        serial,
        "[nvme] controller at {:02x}:{:02x}.{}  BAR0={:#x} size={:#x}",
        dev.bus, dev.device, dev.function, bar0.base, bar0.size,
    );
    dev.enable_mmio_bus_master();
    let mmio = vmm::map_mmio(bar0.base, bar0.size as usize).ok()?;

    let mut ctrl = bring_up(mmio)?;
    identify_controller(&mut ctrl, &mut serial)?;
    let (nsid, lba_bytes, block_count) = identify_namespace(&mut ctrl, &mut serial)?;
    create_io_queues(&mut ctrl, &mut serial)?;

    let controller = Arc::new(Mutex::new(ctrl));
    let ns = Arc::new(NvmeNs { controller, nsid, lba_bytes, block_count });
    let _ = writeln!(
        serial,
        "[nvme] ns {} ready: {} blocks × {} B = {} bytes",
        nsid, block_count, lba_bytes, block_count * lba_bytes as u64,
    );
    let ns_dyn: Arc<dyn BlockDevice> = ns.clone();
    crate::drivers::block::register(ns_dyn.clone());
    Some(ns_dyn)
}

fn bring_up(mmio: u64) -> Option<Controller> {
    let mut ctrl = Controller { mmio, admin: dummy_queue(), io: None, doorbell_stride: 0 };
    // Disable the controller.
    let cc = ctrl.read32(REG_CC);
    if cc & CC_EN != 0 {
        ctrl.write32(REG_CC, cc & !CC_EN);
        while ctrl.read32(REG_CSTS) & CSTS_RDY != 0 {
            core::hint::spin_loop();
        }
    }

    let cap = ctrl.read64(REG_CAP);
    ctrl.doorbell_stride = ((cap >> 32) & 0xF) as u32;
    let mqes = (cap & 0xFFFF) as u32 + 1;
    let adm_depth = ADMIN_Q_DEPTH.min(mqes);

    // Admin queues: one page each.
    let asq = pmm::alloc_frame()?;
    let acq = pmm::alloc_frame()?;
    unsafe {
        core::ptr::write_bytes((asq + HHDM_OFFSET) as *mut u8, 0, PAGE_SIZE);
        core::ptr::write_bytes((acq + HHDM_OFFSET) as *mut u8, 0, PAGE_SIZE);
    }
    ctrl.write32(REG_AQA, ((adm_depth - 1) << 16) | (adm_depth - 1));
    ctrl.write64(REG_ASQ, asq);
    ctrl.write64(REG_ACQ, acq);

    ctrl.admin = Queue {
        sq_phys: asq,
        cq_phys: acq,
        sq_depth: adm_depth,
        cq_depth: adm_depth,
        sq_tail: 0,
        cq_head: 0,
        phase: 1,
        doorbell_sq: ctrl.doorbell(0, false),
        doorbell_cq: ctrl.doorbell(0, true),
        next_cid: 0,
    };

    // Enable.
    let new_cc = CC_IOSQES_6 | CC_IOCQES_4 | CC_EN;
    ctrl.write32(REG_CC, new_cc);
    let mut spins = 0u64;
    while ctrl.read32(REG_CSTS) & CSTS_RDY == 0 {
        core::hint::spin_loop();
        spins += 1;
        if spins > 100_000_000 {
            return None;
        }
    }
    Some(ctrl)
}

fn dummy_queue() -> Queue {
    Queue {
        sq_phys: 0,
        cq_phys: 0,
        sq_depth: 0,
        cq_depth: 0,
        sq_tail: 0,
        cq_head: 0,
        phase: 0,
        doorbell_sq: core::ptr::null_mut(),
        doorbell_cq: core::ptr::null_mut(),
        next_cid: 0,
    }
}

fn identify_controller(ctrl: &mut Controller, serial: &mut SerialPort) -> Option<()> {
    let buf = pmm::alloc_frame()?;
    unsafe { core::ptr::write_bytes((buf + HHDM_OFFSET) as *mut u8, 0, PAGE_SIZE) };
    let mut cmd = SubmissionEntry::zero();
    cmd.cdw0 = 0x06; // Identify
    cmd.nsid = 0;
    cmd.prp1 = buf;
    cmd.cdw10 = 1; // CNS=1 controller
    let (_, status) = ctrl.admin.submit_sync(cmd);
    if status != 0 {
        pmm::free_frame(buf);
        let _ = writeln!(serial, "[nvme] identify controller failed status={:#x}", status);
        return None;
    }
    // VID at offset 0, model name at 24..64 (40 bytes).
    let slice = unsafe { core::slice::from_raw_parts((buf + HHDM_OFFSET) as *const u8, 64) };
    let vid = u16::from_le_bytes([slice[0], slice[1]]);
    let did = u16::from_le_bytes([slice[2], slice[3]]);
    let model_raw = &slice[24..64];
    let model = trim_ascii(model_raw);
    let _ = writeln!(serial, "[nvme] ctrl VID={:04x} DID={:04x} model=\"{}\"", vid, did, model);
    pmm::free_frame(buf);
    Some(())
}

fn trim_ascii(bytes: &[u8]) -> &str {
    let end = bytes.iter().rposition(|&b| b != b' ' && b != 0).map(|i| i + 1).unwrap_or(0);
    core::str::from_utf8(&bytes[..end]).unwrap_or("?")
}

fn identify_namespace(
    ctrl: &mut Controller,
    serial: &mut SerialPort,
) -> Option<(u32, u32, u64)> {
    // Active NSID list.
    let buf = pmm::alloc_frame()?;
    unsafe { core::ptr::write_bytes((buf + HHDM_OFFSET) as *mut u8, 0, PAGE_SIZE) };
    let mut cmd = SubmissionEntry::zero();
    cmd.cdw0 = 0x06;
    cmd.prp1 = buf;
    cmd.cdw10 = 2; // CNS=2 active NSID list
    let (_, status) = ctrl.admin.submit_sync(cmd);
    if status != 0 {
        pmm::free_frame(buf);
        let _ = writeln!(serial, "[nvme] NSID list failed status={:#x}", status);
        return None;
    }
    let nsids = unsafe { core::slice::from_raw_parts((buf + HHDM_OFFSET) as *const u32, 1024) };
    let nsid = nsids.first().copied().unwrap_or(0);
    pmm::free_frame(buf);
    if nsid == 0 {
        let _ = writeln!(serial, "[nvme] no active namespaces");
        return None;
    }

    // Identify the namespace.
    let ns_buf = pmm::alloc_frame()?;
    unsafe { core::ptr::write_bytes((ns_buf + HHDM_OFFSET) as *mut u8, 0, PAGE_SIZE) };
    let mut cmd = SubmissionEntry::zero();
    cmd.cdw0 = 0x06;
    cmd.nsid = nsid;
    cmd.prp1 = ns_buf;
    cmd.cdw10 = 0; // CNS=0 namespace
    let (_, status) = ctrl.admin.submit_sync(cmd);
    if status != 0 {
        pmm::free_frame(ns_buf);
        let _ = writeln!(serial, "[nvme] identify NS failed status={:#x}", status);
        return None;
    }
    let raw = unsafe { core::slice::from_raw_parts((ns_buf + HHDM_OFFSET) as *const u8, 4096) };
    // NSZE at offset 0 (u64), NLBAF at 25, FLBAS at 26, LBAF array starts at 128.
    let nsze = u64::from_le_bytes(raw[0..8].try_into().unwrap());
    let flbas = raw[26] & 0xF;
    let lbaf_off = 128 + (flbas as usize) * 4;
    let lbads = raw[lbaf_off + 2];
    let lba_bytes = 1u32 << lbads;
    pmm::free_frame(ns_buf);
    let _ = writeln!(serial, "[nvme] ns={} blocks={} lba={}B", nsid, nsze, lba_bytes);
    Some((nsid, lba_bytes, nsze))
}

fn create_io_queues(ctrl: &mut Controller, serial: &mut SerialPort) -> Option<()> {
    let cq_phys = pmm::alloc_frame()?;
    let sq_phys = pmm::alloc_frame()?;
    unsafe {
        core::ptr::write_bytes((cq_phys + HHDM_OFFSET) as *mut u8, 0, PAGE_SIZE);
        core::ptr::write_bytes((sq_phys + HHDM_OFFSET) as *mut u8, 0, PAGE_SIZE);
    }
    let depth = IO_Q_DEPTH;

    // Create IO CQ (qid=1). Physically contiguous (PC=1), IRQ disabled.
    let mut cmd = SubmissionEntry::zero();
    cmd.cdw0 = 0x05;
    cmd.prp1 = cq_phys;
    cmd.cdw10 = ((depth - 1) << 16) | 1; // qid=1
    cmd.cdw11 = 1; // PC=1, IEN=0
    let (_, status) = ctrl.admin.submit_sync(cmd);
    if status != 0 {
        let _ = writeln!(serial, "[nvme] create CQ failed status={:#x}", status);
        return None;
    }

    // Create IO SQ (qid=1) bound to CQ 1.
    let mut cmd = SubmissionEntry::zero();
    cmd.cdw0 = 0x01;
    cmd.prp1 = sq_phys;
    cmd.cdw10 = ((depth - 1) << 16) | 1; // qid=1
    cmd.cdw11 = (1 << 16) | 1; // cqid=1, PC=1
    let (_, status) = ctrl.admin.submit_sync(cmd);
    if status != 0 {
        let _ = writeln!(serial, "[nvme] create SQ failed status={:#x}", status);
        return None;
    }

    ctrl.io = Some(Queue {
        sq_phys,
        cq_phys,
        sq_depth: depth,
        cq_depth: depth,
        sq_tail: 0,
        cq_head: 0,
        phase: 1,
        doorbell_sq: ctrl.doorbell(1, false),
        doorbell_cq: ctrl.doorbell(1, true),
        next_cid: 0,
    });
    Some(())
}

/// Hand the block device back so early boot can do a smoke read before VFS
/// takes over the main code path. Unused if M2.4 has already attached it.
#[allow(dead_code)]
pub fn list_registered_devices() -> Vec<Arc<dyn BlockDevice>> {
    let mut v = Vec::new();
    if let Some(d) = crate::drivers::block::first() {
        v.push(d);
    }
    v
}
