//! Host-side PondFS formatter.
//!
//! Usage: `mkpondfs <image> <source-dir>`
//!
//! The image is treated as a raw, block-addressable device. We write a
//! superblock + bitmap + inode table at the start, then populate the data
//! region with the files found under `<source-dir>`. Directory nesting IS
//! supported, though the kernel currently expects a flat root.

use std::env;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::exit;

const MAGIC: [u8; 4] = *b"POND";
const VERSION: u32 = 1;
const BLOCK_SIZE: usize = 4096;
const INODE_SIZE: usize = 128;
const INODES_PER_BLOCK: usize = BLOCK_SIZE / INODE_SIZE;
const DIRECT_BLOCKS: usize = 12;

const KIND_UNUSED: u16 = 0;
const KIND_FILE: u16 = 1;
const KIND_DIR: u16 = 2;

const ROOT_INO: u64 = 1;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Superblock {
    magic: [u8; 4],
    version: u32,
    total_blocks: u64,
    inode_count: u64,
    bitmap_start: u64,
    bitmap_blocks: u64,
    inode_table_start: u64,
    inode_table_blocks: u64,
    data_start: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RawInode {
    kind: u16,
    _pad0: u16,
    link_count: u32,
    size: u64,
    mtime: u64,
    _pad1: u64,
    direct: [u64; DIRECT_BLOCKS],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct DirEntryHeader {
    inode: u64,
    name_len: u16,
    kind: u16,
}

const DIR_HDR_SIZE: usize = std::mem::size_of::<DirEntryHeader>();

struct Formatter {
    img: File,
    total_blocks: u64,
    inode_count: u64,
    inodes: Vec<RawInode>,
    // data-block bitmap, 1 bit per data block (LSB first).
    bitmap: Vec<u8>,
    bitmap_start: u64,
    bitmap_blocks: u64,
    inode_table_start: u64,
    inode_table_blocks: u64,
    data_start: u64,
}

impl Formatter {
    fn new(img: File, total_bytes: u64, inode_count: u64) -> Self {
        let total_blocks = total_bytes / BLOCK_SIZE as u64;
        assert!(total_blocks >= 64, "image too small");

        // Layout:
        //   block 0                       : superblock
        //   block 1..=bitmap_blocks       : bitmap
        //   then inode table blocks
        //   then data blocks
        let inode_table_blocks = inode_count.div_ceil(INODES_PER_BLOCK as u64);

        // Bitmap sizing: worst case covers all remaining blocks.
        // Iterate to converge on a sane split.
        let mut bitmap_blocks = 1u64;
        let mut data_blocks;
        loop {
            let reserved = 1 + bitmap_blocks + inode_table_blocks;
            data_blocks = total_blocks.saturating_sub(reserved);
            let need = data_blocks.div_ceil(8 * BLOCK_SIZE as u64);
            if need <= bitmap_blocks { break; }
            bitmap_blocks = need;
        }

        let bitmap_start = 1;
        let inode_table_start = bitmap_start + bitmap_blocks;
        let data_start = inode_table_start + inode_table_blocks;

        let bitmap = vec![0u8; (bitmap_blocks as usize) * BLOCK_SIZE];
        let inodes = vec![RawInode::default(); inode_count as usize];

        Self {
            img,
            total_blocks,
            inode_count,
            inodes,
            bitmap,
            bitmap_start,
            bitmap_blocks,
            inode_table_start,
            inode_table_blocks,
            data_start,
        }
    }

    fn alloc_data_block(&mut self) -> u64 {
        let data_blocks = self.total_blocks - self.data_start;
        for i in 0..data_blocks as usize {
            let byte = i / 8;
            let bit = i % 8;
            if self.bitmap[byte] & (1 << bit) == 0 {
                self.bitmap[byte] |= 1 << bit;
                return self.data_start + i as u64;
            }
        }
        panic!("out of data blocks");
    }

    fn alloc_inode(&mut self) -> u64 {
        for (i, inode) in self.inodes.iter().enumerate().skip(1) {
            if inode.kind == KIND_UNUSED {
                return i as u64;
            }
        }
        panic!("out of inodes");
    }

    fn write_block(&mut self, blk: u64, data: &[u8; BLOCK_SIZE]) {
        self.img
            .seek(SeekFrom::Start(blk * BLOCK_SIZE as u64))
            .expect("seek");
        self.img.write_all(data).expect("write");
    }

    fn write_file_bytes(&mut self, ino: u64, bytes: &[u8]) {
        let needed = bytes.len().div_ceil(BLOCK_SIZE);
        assert!(needed <= DIRECT_BLOCKS, "file {} too large", ino);
        let mut direct = [0u64; DIRECT_BLOCKS];
        for (i, chunk) in bytes.chunks(BLOCK_SIZE).enumerate() {
            let blk = self.alloc_data_block();
            let mut buf = [0u8; BLOCK_SIZE];
            buf[..chunk.len()].copy_from_slice(chunk);
            self.write_block(blk, &buf);
            direct[i] = blk;
        }
        let inode = &mut self.inodes[ino as usize];
        inode.kind = KIND_FILE;
        inode.link_count = 1;
        inode.size = bytes.len() as u64;
        inode.direct = direct;
    }

    fn write_directory(&mut self, ino: u64, entries: &[(String, u16, u64)]) {
        // Serialize entries.
        let mut buf: Vec<u8> = Vec::new();
        for (name, kind, ino) in entries {
            let hdr = DirEntryHeader {
                inode: *ino,
                name_len: name.len() as u16,
                kind: *kind,
            };
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    (&hdr) as *const _ as *const u8,
                    DIR_HDR_SIZE,
                )
            };
            buf.extend_from_slice(bytes);
            buf.extend_from_slice(name.as_bytes());
            while buf.len() % 4 != 0 {
                buf.push(0);
            }
        }
        self.write_file_bytes(ino, &buf);
        // write_file_bytes sets kind=FILE; fix to DIR.
        self.inodes[ino as usize].kind = KIND_DIR;
    }

    fn finalize(&mut self) {
        // Write inode table.
        for blk_off in 0..self.inode_table_blocks {
            let blk = self.inode_table_start + blk_off;
            let mut buf = [0u8; BLOCK_SIZE];
            for i in 0..INODES_PER_BLOCK {
                let idx = (blk_off as usize) * INODES_PER_BLOCK + i;
                if idx >= self.inodes.len() { break; }
                unsafe {
                    *(buf.as_mut_ptr().add(i * INODE_SIZE) as *mut RawInode) = self.inodes[idx];
                }
            }
            self.write_block(blk, &buf);
        }
        // Write bitmap.
        for blk_off in 0..self.bitmap_blocks {
            let blk = self.bitmap_start + blk_off;
            let mut buf = [0u8; BLOCK_SIZE];
            let src = (blk_off as usize) * BLOCK_SIZE;
            buf.copy_from_slice(&self.bitmap[src..src + BLOCK_SIZE]);
            self.write_block(blk, &buf);
        }
        // Write superblock last.
        let sb = Superblock {
            magic: MAGIC,
            version: VERSION,
            total_blocks: self.total_blocks,
            inode_count: self.inode_count,
            bitmap_start: self.bitmap_start,
            bitmap_blocks: self.bitmap_blocks,
            inode_table_start: self.inode_table_start,
            inode_table_blocks: self.inode_table_blocks,
            data_start: self.data_start,
        };
        let mut buf = [0u8; BLOCK_SIZE];
        unsafe {
            *(buf.as_mut_ptr() as *mut Superblock) = sb;
        }
        self.write_block(0, &buf);
    }
}

fn visit(fmt: &mut Formatter, dir_ino: u64, src: &Path) {
    let mut entries: Vec<(String, u16, u64)> = Vec::new();
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    for e in fs::read_dir(src).expect("read_dir") {
        let e = e.expect("dirent");
        let name = e.file_name().to_string_lossy().into_owned();
        let meta = e.metadata().expect("meta");
        if meta.is_dir() {
            let new_ino = fmt.alloc_inode();
            fmt.inodes[new_ino as usize].kind = KIND_DIR;
            fmt.inodes[new_ino as usize].link_count = 1;
            entries.push((name.clone(), KIND_DIR, new_ino));
            visit(fmt, new_ino, &e.path());
        } else if meta.is_file() {
            let ino = fmt.alloc_inode();
            // Mark as FILE now so a subsequent alloc_inode won't reuse it.
            fmt.inodes[ino as usize].kind = KIND_FILE;
            fmt.inodes[ino as usize].link_count = 1;
            files.push((e.path(), name.clone()));
            entries.push((name, KIND_FILE, ino));
        }
    }
    // Write file bodies now that all inodes are assigned.
    for (path, name) in files {
        // Find the assigned inode in our `entries`.
        let ino = entries.iter().find(|(n, _, _)| n == &name).unwrap().2;
        let mut f = File::open(path).expect("open src file");
        let mut data = Vec::new();
        f.read_to_end(&mut data).expect("read");
        fmt.write_file_bytes(ino, &data);
    }
    fmt.write_directory(dir_ino, &entries);
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: mkpondfs <image> <source-dir>");
        exit(2);
    }
    let img_path = &args[1];
    let src_dir = Path::new(&args[2]);
    if !src_dir.is_dir() {
        eprintln!("source '{}' is not a directory", src_dir.display());
        exit(2);
    }

    let img = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(img_path)
        .expect("open image");
    let size = img.metadata().expect("meta").len();
    let mut fmt = Formatter::new(img, size, 64);

    // Reserve root inode.
    fmt.inodes[ROOT_INO as usize].kind = KIND_DIR;
    fmt.inodes[ROOT_INO as usize].link_count = 1;

    visit(&mut fmt, ROOT_INO, src_dir);

    fmt.finalize();
    eprintln!(
        "pondfs: {} blocks, {} inodes, bitmap@{}+{}, inodes@{}+{}, data@{}",
        fmt.total_blocks, fmt.inode_count,
        fmt.bitmap_start, fmt.bitmap_blocks,
        fmt.inode_table_start, fmt.inode_table_blocks,
        fmt.data_start,
    );
}
