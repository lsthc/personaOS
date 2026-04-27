//! PondFS — simple inode + bitmap + direct-block filesystem.
//!
//! Mounted read-write. Design is intentionally boring: one superblock, one
//! flat inode table, one free-block bitmap, 12 direct blocks per inode.
//! Good enough to host the init ELF and the user's scratch files until
//! we need journaling or larger files (next milestone).

pub mod layout;

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use spin::Mutex;

use crate::drivers::block::BlockDevice;
use crate::fs::{Filesystem, FsError, Inode, InodeKind};

use layout::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn lbas_per_block(dev: &dyn BlockDevice) -> u64 {
    (BLOCK_SIZE / dev.block_size()) as u64
}

fn read_block(dev: &dyn BlockDevice, blk: u64, buf: &mut [u8; BLOCK_SIZE]) -> Result<(), FsError> {
    dev.read_blocks(blk * lbas_per_block(dev), buf)
        .map_err(|_| FsError::Io)
}

fn write_block(dev: &dyn BlockDevice, blk: u64, buf: &[u8; BLOCK_SIZE]) -> Result<(), FsError> {
    dev.write_blocks(blk * lbas_per_block(dev), buf)
        .map_err(|_| FsError::Io)
}

// ---------------------------------------------------------------------------
// Filesystem
// ---------------------------------------------------------------------------

struct FsInner {
    dev: Arc<dyn BlockDevice>,
    sb: Superblock,
    bitmap: Vec<u8>, // in-memory copy; dirty-flushed on changes
    inode_cache: BTreeMap<u64, Arc<InodeState>>,
}

pub struct PondFs {
    #[allow(dead_code)] // kept alive so root's weak references remain valid
    inner: Arc<Mutex<FsInner>>,
    root: Arc<PondInode>,
}

struct InodeState {
    ino: u64,
    raw: Mutex<RawInode>,
}

pub struct PondInode {
    fs: Arc<Mutex<FsInner>>,
    state: Arc<InodeState>,
}

impl PondFs {
    pub fn mount(dev: Arc<dyn BlockDevice>) -> Result<Arc<Self>, FsError> {
        let mut sb_buf = [0u8; BLOCK_SIZE];
        read_block(&*dev, 0, &mut sb_buf)?;
        let sb = unsafe { *(sb_buf.as_ptr() as *const Superblock) };
        if sb.magic != POND_MAGIC {
            return Err(FsError::Io);
        }
        if sb.version != POND_VERSION {
            return Err(FsError::Io);
        }

        // Read the entire bitmap into memory (small for M2).
        let mut bitmap = vec![0u8; (sb.bitmap_blocks as usize) * BLOCK_SIZE];
        for i in 0..sb.bitmap_blocks {
            let mut buf = [0u8; BLOCK_SIZE];
            read_block(&*dev, sb.bitmap_start + i, &mut buf)?;
            let start = (i as usize) * BLOCK_SIZE;
            bitmap[start..start + BLOCK_SIZE].copy_from_slice(&buf);
        }

        let inner = Arc::new(Mutex::new(FsInner {
            dev,
            sb,
            bitmap,
            inode_cache: BTreeMap::new(),
        }));

        let root_state = load_inode(&inner, ROOT_INO)?;
        {
            let mut g = inner.lock();
            g.inode_cache.insert(ROOT_INO, root_state.clone());
        }
        let root = Arc::new(PondInode {
            fs: inner.clone(),
            state: root_state,
        });

        Ok(Arc::new(Self { inner, root }))
    }
}

impl Filesystem for PondFs {
    fn root(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }
}

// ---------------------------------------------------------------------------
// Inode I/O
// ---------------------------------------------------------------------------

fn load_inode(fs: &Arc<Mutex<FsInner>>, ino: u64) -> Result<Arc<InodeState>, FsError> {
    let g = fs.lock();
    if ino == 0 || ino >= g.sb.inode_count {
        return Err(FsError::NotFound);
    }
    let per_block = INODES_PER_BLOCK as u64;
    let blk = g.sb.inode_table_start + ino / per_block;
    let idx = (ino % per_block) as usize;
    let dev = g.dev.clone();
    drop(g);

    let mut buf = [0u8; BLOCK_SIZE];
    read_block(&*dev, blk, &mut buf)?;
    let raw = unsafe { *(buf.as_ptr().add(idx * INODE_SIZE) as *const RawInode) };
    Ok(Arc::new(InodeState {
        ino,
        raw: Mutex::new(raw),
    }))
}

fn write_inode(fs: &FsInner, state: &InodeState) -> Result<(), FsError> {
    let per_block = INODES_PER_BLOCK as u64;
    let blk = fs.sb.inode_table_start + state.ino / per_block;
    let idx = (state.ino % per_block) as usize;
    let mut buf = [0u8; BLOCK_SIZE];
    read_block(&*fs.dev, blk, &mut buf)?;
    unsafe {
        *(buf.as_mut_ptr().add(idx * INODE_SIZE) as *mut RawInode) = *state.raw.lock();
    }
    write_block(&*fs.dev, blk, &buf)?;
    Ok(())
}

fn get_or_load(fs: &Arc<Mutex<FsInner>>, ino: u64) -> Result<Arc<InodeState>, FsError> {
    {
        let g = fs.lock();
        if let Some(s) = g.inode_cache.get(&ino) {
            return Ok(s.clone());
        }
    }
    let s = load_inode(fs, ino)?;
    fs.lock().inode_cache.insert(ino, s.clone());
    Ok(s)
}

// ---------------------------------------------------------------------------
// Bitmap (data block allocator)
// ---------------------------------------------------------------------------

fn alloc_data_block(fs: &mut FsInner) -> Result<u64, FsError> {
    let data_blocks = fs.sb.total_blocks - fs.sb.data_start;
    for i in 0..data_blocks as usize {
        let byte = i / 8;
        let bit = i % 8;
        if fs.bitmap[byte] & (1 << bit) == 0 {
            fs.bitmap[byte] |= 1 << bit;
            let bitmap_blk = fs.sb.bitmap_start + (byte / BLOCK_SIZE) as u64;
            let offset_in_blk = byte % BLOCK_SIZE;
            // Flush the one bitmap block we touched.
            let mut buf = [0u8; BLOCK_SIZE];
            read_block(&*fs.dev, bitmap_blk, &mut buf)?;
            buf[offset_in_blk] = fs.bitmap[byte];
            write_block(&*fs.dev, bitmap_blk, &buf)?;
            return Ok(fs.sb.data_start + i as u64);
        }
    }
    Err(FsError::NoSpace)
}

fn free_data_block(fs: &mut FsInner, blk: u64) -> Result<(), FsError> {
    if blk < fs.sb.data_start || blk >= fs.sb.total_blocks {
        return Err(FsError::Io);
    }
    let i = (blk - fs.sb.data_start) as usize;
    let byte = i / 8;
    let bit = i % 8;
    fs.bitmap[byte] &= !(1 << bit);
    let bitmap_blk = fs.sb.bitmap_start + (byte / BLOCK_SIZE) as u64;
    let offset_in_blk = byte % BLOCK_SIZE;
    let mut buf = [0u8; BLOCK_SIZE];
    read_block(&*fs.dev, bitmap_blk, &mut buf)?;
    buf[offset_in_blk] = fs.bitmap[byte];
    write_block(&*fs.dev, bitmap_blk, &buf)?;
    Ok(())
}

fn alloc_inode(fs: &mut FsInner) -> Result<u64, FsError> {
    // Scan the inode table for a KIND_UNUSED slot.
    let per_block = INODES_PER_BLOCK as u64;
    for blk_off in 0..fs.sb.inode_table_blocks {
        let blk = fs.sb.inode_table_start + blk_off;
        let mut buf = [0u8; BLOCK_SIZE];
        read_block(&*fs.dev, blk, &mut buf)?;
        for i in 0..INODES_PER_BLOCK {
            let ino = blk_off * per_block + i as u64;
            if ino == 0 {
                continue;
            } // reserve inode 0
            let raw = unsafe { &*(buf.as_ptr().add(i * INODE_SIZE) as *const RawInode) };
            if raw.kind == INODE_KIND_UNUSED {
                return Ok(ino);
            }
        }
    }
    Err(FsError::NoSpace)
}

// ---------------------------------------------------------------------------
// File data I/O via direct blocks only (max 48 KiB).
// ---------------------------------------------------------------------------

fn read_file_at(fs: &FsInner, raw: &RawInode, off: u64, buf: &mut [u8]) -> Result<usize, FsError> {
    if off >= raw.size {
        return Ok(0);
    }
    let end = core::cmp::min(raw.size, off + buf.len() as u64);
    let mut cursor = off;
    let mut written = 0usize;
    while cursor < end {
        let blk_idx = (cursor / BLOCK_SIZE as u64) as usize;
        if blk_idx >= DIRECT_BLOCKS {
            return Err(FsError::Unsupported);
        }
        let blk = raw.direct[blk_idx];
        if blk == 0 {
            return Err(FsError::Io);
        }
        let mut blk_buf = [0u8; BLOCK_SIZE];
        read_block(&*fs.dev, blk, &mut blk_buf)?;
        let in_blk = (cursor % BLOCK_SIZE as u64) as usize;
        let remain = (end - cursor) as usize;
        let take = core::cmp::min(BLOCK_SIZE - in_blk, remain);
        buf[written..written + take].copy_from_slice(&blk_buf[in_blk..in_blk + take]);
        written += take;
        cursor += take as u64;
    }
    Ok(written)
}

fn write_file_at(
    fs: &mut FsInner,
    raw: &mut RawInode,
    off: u64,
    buf: &[u8],
) -> Result<usize, FsError> {
    let end = off + buf.len() as u64;
    let needed_blocks = end.div_ceil(BLOCK_SIZE as u64) as usize;
    if needed_blocks > DIRECT_BLOCKS {
        return Err(FsError::Unsupported);
    }
    // Allocate any missing direct blocks.
    for i in 0..needed_blocks {
        if raw.direct[i] == 0 {
            let nb = alloc_data_block(fs)?;
            // Zero the new block before we write partial data into it.
            let zeros = [0u8; BLOCK_SIZE];
            write_block(&*fs.dev, nb, &zeros)?;
            raw.direct[i] = nb;
        }
    }

    let mut cursor = off;
    let mut src = 0usize;
    while src < buf.len() {
        let blk_idx = (cursor / BLOCK_SIZE as u64) as usize;
        let blk = raw.direct[blk_idx];
        let mut blk_buf = [0u8; BLOCK_SIZE];
        read_block(&*fs.dev, blk, &mut blk_buf)?;
        let in_blk = (cursor % BLOCK_SIZE as u64) as usize;
        let take = core::cmp::min(BLOCK_SIZE - in_blk, buf.len() - src);
        blk_buf[in_blk..in_blk + take].copy_from_slice(&buf[src..src + take]);
        write_block(&*fs.dev, blk, &blk_buf)?;
        src += take;
        cursor += take as u64;
    }
    if end > raw.size {
        raw.size = end;
    }
    Ok(buf.len())
}

// ---------------------------------------------------------------------------
// Directory ops
// ---------------------------------------------------------------------------

fn dir_read_entries(
    fs: &FsInner,
    raw: &RawInode,
) -> Result<Vec<(String, InodeKind, u64)>, FsError> {
    let mut entries = Vec::new();
    let mut buf = vec![0u8; raw.size as usize];
    read_file_at(fs, raw, 0, &mut buf)?;
    let mut i = 0usize;
    while i + DIR_ENTRY_HEADER_SIZE <= buf.len() {
        let hdr: DirEntryHeader =
            unsafe { core::ptr::read_unaligned(buf.as_ptr().add(i) as *const DirEntryHeader) };
        if hdr.name_len == 0 {
            break;
        }
        let name_start = i + DIR_ENTRY_HEADER_SIZE;
        let name_end = name_start + hdr.name_len as usize;
        if name_end > buf.len() {
            return Err(FsError::Io);
        }
        let name = core::str::from_utf8(&buf[name_start..name_end])
            .map_err(|_| FsError::Io)?
            .to_string();
        let kind = match hdr.kind {
            INODE_KIND_FILE => InodeKind::File,
            INODE_KIND_DIR => InodeKind::Dir,
            _ => return Err(FsError::Io),
        };
        entries.push((name, kind, hdr.inode));
        i = name_end;
        // pad to 4 bytes
        i = align4(i);
    }
    Ok(entries)
}

fn dir_write_entries(
    fs: &mut FsInner,
    raw: &mut RawInode,
    entries: &[(String, InodeKind, u64)],
) -> Result<(), FsError> {
    // Serialize into a byte buffer.
    let mut buf: Vec<u8> = Vec::new();
    for (name, kind, ino) in entries {
        let hdr = DirEntryHeader {
            inode: *ino,
            name_len: name.len() as u16,
            kind: match kind {
                InodeKind::File => INODE_KIND_FILE,
                InodeKind::Dir => INODE_KIND_DIR,
            },
        };
        let off = buf.len();
        buf.resize(off + DIR_ENTRY_HEADER_SIZE, 0);
        unsafe {
            core::ptr::write_unaligned(buf.as_mut_ptr().add(off) as *mut DirEntryHeader, hdr);
        }
        buf.extend_from_slice(name.as_bytes());
        while !buf.len().is_multiple_of(4) {
            buf.push(0);
        }
    }
    // Free any excess direct blocks beyond what the new size needs.
    let old_blocks = (raw.size as usize).div_ceil(BLOCK_SIZE);
    let new_blocks = buf.len().div_ceil(BLOCK_SIZE);
    for i in new_blocks..old_blocks.min(DIRECT_BLOCKS) {
        if raw.direct[i] != 0 {
            free_data_block(fs, raw.direct[i])?;
            raw.direct[i] = 0;
        }
    }
    raw.size = 0;
    write_file_at(fs, raw, 0, &buf)?;
    raw.size = buf.len() as u64;
    Ok(())
}

// ---------------------------------------------------------------------------
// Inode trait impl
// ---------------------------------------------------------------------------

impl Inode for PondInode {
    fn kind(&self) -> InodeKind {
        let raw = self.state.raw.lock();
        if raw.kind == INODE_KIND_DIR {
            InodeKind::Dir
        } else {
            InodeKind::File
        }
    }

    fn size(&self) -> u64 {
        self.state.raw.lock().size
    }

    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        let fs = self.fs.lock();
        let raw = self.state.raw.lock();
        if raw.kind != INODE_KIND_FILE {
            return Err(FsError::IsDir);
        }
        read_file_at(&fs, &raw, off, buf)
    }

    fn write_at(&self, off: u64, buf: &[u8]) -> Result<usize, FsError> {
        let mut fs = self.fs.lock();
        let mut raw = self.state.raw.lock();
        if raw.kind != INODE_KIND_FILE {
            return Err(FsError::IsDir);
        }
        let n = write_file_at(&mut fs, &mut raw, off, buf)?;
        write_inode(
            &fs,
            &InodeState {
                ino: self.state.ino,
                raw: Mutex::new(*raw),
            },
        )?;
        Ok(n)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> {
        let raw_copy = *self.state.raw.lock();
        if raw_copy.kind != INODE_KIND_DIR {
            return Err(FsError::NotDir);
        }
        let fs = self.fs.lock();
        let entries = dir_read_entries(&fs, &raw_copy)?;
        drop(fs);
        for (n, _, ino) in entries {
            if n == name {
                let state = get_or_load(&self.fs, ino)?;
                return Ok(Arc::new(PondInode {
                    fs: self.fs.clone(),
                    state,
                }));
            }
        }
        Err(FsError::NotFound)
    }

    fn readdir(&self) -> Result<Vec<(String, InodeKind)>, FsError> {
        let raw_copy = *self.state.raw.lock();
        if raw_copy.kind != INODE_KIND_DIR {
            return Err(FsError::NotDir);
        }
        let fs = self.fs.lock();
        let entries = dir_read_entries(&fs, &raw_copy)?;
        Ok(entries.into_iter().map(|(n, k, _)| (n, k)).collect())
    }

    fn create(&self, name: &str, kind: InodeKind) -> Result<Arc<dyn Inode>, FsError> {
        let mut fs = self.fs.lock();
        let mut parent_raw = self.state.raw.lock();
        if parent_raw.kind != INODE_KIND_DIR {
            return Err(FsError::NotDir);
        }
        let mut entries = dir_read_entries(&fs, &parent_raw)?;
        if entries.iter().any(|(n, _, _)| n == name) {
            return Err(FsError::Exists);
        }

        // Allocate an inode slot.
        let new_ino = alloc_inode(&mut fs)?;
        let new_raw = RawInode {
            kind: match kind {
                InodeKind::File => INODE_KIND_FILE,
                InodeKind::Dir => INODE_KIND_DIR,
            },
            _pad0: 0,
            link_count: 1,
            size: 0,
            mtime: 0,
            _pad1: 0,
            direct: [0; DIRECT_BLOCKS],
        };
        let state = Arc::new(InodeState {
            ino: new_ino,
            raw: Mutex::new(new_raw),
        });
        write_inode(&fs, &state)?;

        // Update parent directory.
        entries.push((String::from(name), kind, new_ino));
        dir_write_entries(&mut fs, &mut parent_raw, &entries)?;
        write_inode(
            &fs,
            &InodeState {
                ino: self.state.ino,
                raw: Mutex::new(*parent_raw),
            },
        )?;

        fs.inode_cache.insert(new_ino, state.clone());
        Ok(Arc::new(PondInode {
            fs: self.fs.clone(),
            state,
        }))
    }

    fn unlink(&self, name: &str) -> Result<(), FsError> {
        let mut fs = self.fs.lock();
        let mut parent_raw = self.state.raw.lock();
        if parent_raw.kind != INODE_KIND_DIR {
            return Err(FsError::NotDir);
        }
        let mut entries = dir_read_entries(&fs, &parent_raw)?;
        let pos = entries
            .iter()
            .position(|(n, _, _)| n == name)
            .ok_or(FsError::NotFound)?;
        let (_, _, ino) = entries.remove(pos);

        // Free the unlinked inode's data blocks and mark it unused.
        let state = get_or_load(&self.fs, ino)?;
        let mut raw = state.raw.lock();
        for i in 0..DIRECT_BLOCKS {
            if raw.direct[i] != 0 {
                free_data_block(&mut fs, raw.direct[i])?;
                raw.direct[i] = 0;
            }
        }
        raw.kind = INODE_KIND_UNUSED;
        raw.size = 0;
        raw.link_count = 0;
        let snap = *raw;
        drop(raw);
        write_inode(
            &fs,
            &InodeState {
                ino,
                raw: Mutex::new(snap),
            },
        )?;
        fs.inode_cache.remove(&ino);

        dir_write_entries(&mut fs, &mut parent_raw, &entries)?;
        write_inode(
            &fs,
            &InodeState {
                ino: self.state.ino,
                raw: Mutex::new(*parent_raw),
            },
        )?;
        Ok(())
    }
}
