//! PondFS on-disk constants and raw structures.
//!
//! Keep this file free of kernel dependencies so `tools/mkpondfs` can share
//! it via `#[path]`-include from the host side.

#![allow(dead_code)]

pub const POND_MAGIC: [u8; 4] = *b"POND";
pub const POND_VERSION: u32 = 1;

pub const BLOCK_SIZE: usize = 4096;
pub const INODE_SIZE: usize = 128;
pub const INODES_PER_BLOCK: usize = BLOCK_SIZE / INODE_SIZE;
pub const DIRECT_BLOCKS: usize = 12;

pub const INODE_KIND_UNUSED: u16 = 0;
pub const INODE_KIND_FILE: u16 = 1;
pub const INODE_KIND_DIR: u16 = 2;

pub const ROOT_INO: u64 = 1;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Superblock {
    pub magic: [u8; 4],
    pub version: u32,
    pub total_blocks: u64,
    pub inode_count: u64,
    pub bitmap_start: u64,
    pub bitmap_blocks: u64,
    pub inode_table_start: u64,
    pub inode_table_blocks: u64,
    pub data_start: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RawInode {
    pub kind: u16,
    pub _pad0: u16,
    pub link_count: u32,
    pub size: u64,
    pub mtime: u64,
    pub _pad1: u64,
    pub direct: [u64; DIRECT_BLOCKS],
}

const _: () = assert!(core::mem::size_of::<RawInode>() == INODE_SIZE);

/// Directory entry layout; `name` is not captured here — callers read the
/// header then a name of `name_len` bytes, then pad to 4-byte boundary.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DirEntryHeader {
    pub inode: u64,
    pub name_len: u16,
    pub kind: u16,
}

pub const DIR_ENTRY_HEADER_SIZE: usize = core::mem::size_of::<DirEntryHeader>();

pub fn align4(x: usize) -> usize {
    (x + 3) & !3
}
