//! Virtual filesystem — traits, mount table, path resolution, FD table.
//!
//! The VFS is intentionally small for M2. It exposes a single Inode trait
//! that every filesystem implementation (ramfs, pondfs) satisfies, and a
//! flat mount table keyed by absolute path prefix. The FD table lives on
//! each Task and stores open files.

pub mod elf;
pub mod pondfs;
pub mod ramfs;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::AtomicU64;

use spin::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InodeKind {
    File,
    Dir,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // consumed by pondfs and userspace as filesystems mature
pub enum FsError {
    NotFound,
    NotDir,
    NotFile,
    Exists,
    InvalidPath,
    IsDir,
    Io,
    NoSpace,
    BadFd,
    Unsupported,
}

#[allow(dead_code)] // readdir / unlink consumed by userspace as soon as there's a shell
pub trait Inode: Send + Sync {
    fn kind(&self) -> InodeKind;
    fn size(&self) -> u64;
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<usize, FsError>;
    fn write_at(&self, off: u64, buf: &[u8]) -> Result<usize, FsError>;
    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>, FsError>;
    fn readdir(&self) -> Result<Vec<(String, InodeKind)>, FsError>;
    fn create(&self, name: &str, kind: InodeKind) -> Result<Arc<dyn Inode>, FsError>;
    fn unlink(&self, name: &str) -> Result<(), FsError>;
}

pub trait Filesystem: Send + Sync {
    fn root(&self) -> Arc<dyn Inode>;
}

// ---------------------------------------------------------------------------
// Mount table
// ---------------------------------------------------------------------------

static MOUNTS: Mutex<Vec<(String, Arc<dyn Filesystem>)>> = Mutex::new(Vec::new());

pub fn mount(path: &str, fs: Arc<dyn Filesystem>) {
    let mut m = MOUNTS.lock();
    // Replace an existing mount at the same path.
    if let Some(slot) = m.iter_mut().find(|(p, _)| p == path) {
        slot.1 = fs;
        return;
    }
    m.push((String::from(path), fs));
    // Longest prefix wins — keep sorted by descending length.
    m.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
}

/// Find the filesystem whose mountpoint is the longest prefix of `path` and
/// return (fs, remaining_path_after_mount).
fn resolve_mount(path: &str) -> Option<(Arc<dyn Filesystem>, String)> {
    let m = MOUNTS.lock();
    for (mp, fs) in m.iter() {
        if path == mp.as_str() {
            return Some((fs.clone(), String::from("/")));
        }
        if path.starts_with(mp.as_str())
            && (mp.ends_with('/') || path.as_bytes().get(mp.len()) == Some(&b'/'))
        {
            let rest = &path[mp.len()..];
            return Some((
                fs.clone(),
                String::from(if rest.is_empty() { "/" } else { rest }),
            ));
        }
    }
    None
}

/// Walk an absolute path and return the target inode.
pub fn lookup(path: &str) -> Result<Arc<dyn Inode>, FsError> {
    if !path.starts_with('/') {
        return Err(FsError::InvalidPath);
    }
    let (fs, rest) = resolve_mount(path).ok_or(FsError::NotFound)?;
    let mut node = fs.root();
    for comp in rest.split('/').filter(|c| !c.is_empty()) {
        node = node.lookup(comp)?;
    }
    Ok(node)
}

/// Split `path` into (parent_path, final_name). For `/foo/bar` returns
/// (`/foo`, `bar`). For `/foo` returns (`/`, `foo`).
pub fn split_parent(path: &str) -> Option<(&str, &str)> {
    if !path.starts_with('/') {
        return None;
    }
    let idx = path.rfind('/')?;
    let name = &path[idx + 1..];
    if name.is_empty() {
        return None;
    }
    let parent = if idx == 0 { "/" } else { &path[..idx] };
    Some((parent, name))
}

// ---------------------------------------------------------------------------
// Open files + FD table
// ---------------------------------------------------------------------------

pub struct OpenFile {
    pub inode: Arc<dyn Inode>,
    pub offset: AtomicU64,
    /// OR of `FD_READ`/`FD_WRITE`/`FD_CREATE`. Not enforced yet — stored for
    /// future permission checks.
    #[allow(dead_code)]
    pub flags: u32,
}

impl OpenFile {
    pub fn new(inode: Arc<dyn Inode>, flags: u32) -> Arc<Self> {
        Arc::new(Self {
            inode,
            offset: AtomicU64::new(0),
            flags,
        })
    }
}

pub const FD_READ: u32 = 1 << 0;
pub const FD_WRITE: u32 = 1 << 1;
#[allow(dead_code)]
pub const FD_CREATE: u32 = 1 << 2;

pub struct FdTable {
    slots: BTreeMap<i32, Arc<OpenFile>>,
    next: i32,
}

impl Default for FdTable {
    fn default() -> Self {
        Self::new()
    }
}

impl FdTable {
    pub const fn new() -> Self {
        Self {
            slots: BTreeMap::new(),
            next: 0,
        }
    }

    pub fn install(&mut self, open: Arc<OpenFile>) -> i32 {
        let fd = self.next;
        self.next += 1;
        self.slots.insert(fd, open);
        fd
    }

    /// Force a specific fd slot — used to wire fd 0/1/2 to a stdio shim when
    /// a new task is spawned.
    pub fn install_at(&mut self, fd: i32, open: Arc<OpenFile>) {
        self.slots.insert(fd, open);
        if fd >= self.next {
            self.next = fd + 1;
        }
    }

    pub fn get(&self, fd: i32) -> Option<Arc<OpenFile>> {
        self.slots.get(&fd).cloned()
    }

    pub fn close(&mut self, fd: i32) -> bool {
        self.slots.remove(&fd).is_some()
    }
}

pub struct TtyStdin;

impl Inode for TtyStdin {
    fn kind(&self) -> InodeKind {
        InodeKind::File
    }
    fn size(&self) -> u64 {
        0
    }
    fn read_at(&self, _off: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        let mut n = 0;
        while n < buf.len() {
            let b = crate::drivers::ps2::read_byte_blocking();
            match b {
                b'\n' => {
                    buf[n] = b'\n';
                    n += 1;
                    tty_echo(b'\n');
                    break;
                }
                0x08 => {
                    if n > 0 {
                        n -= 1;
                        tty_echo(0x08);
                    }
                }
                b if (0x20..=0x7E).contains(&b) || b == b'\t' => {
                    buf[n] = b;
                    n += 1;
                    tty_echo(b);
                }
                _ => {}
            }
        }
        Ok(n)
    }
    fn write_at(&self, _off: u64, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::Unsupported)
    }
    fn lookup(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotDir)
    }
    fn readdir(&self) -> Result<Vec<(String, InodeKind)>, FsError> {
        Err(FsError::NotDir)
    }
    fn create(&self, _name: &str, _kind: InodeKind) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotDir)
    }
    fn unlink(&self, _name: &str) -> Result<(), FsError> {
        Err(FsError::NotDir)
    }
}

fn tty_echo(b: u8) {
    let mut serial = unsafe { crate::drivers::serial::SerialPort::new(0x3F8) };
    match b {
        b'\n' => {
            serial.write_byte(b'\r');
            serial.write_byte(b'\n');
        }
        0x08 => {
            serial.write_byte(0x08);
            serial.write_byte(b' ');
            serial.write_byte(0x08);
        }
        b => serial.write_byte(b),
    }
}

// The stdout/stderr inode: writing goes to the serial console.
pub struct SerialStdout;

impl Inode for SerialStdout {
    fn kind(&self) -> InodeKind {
        InodeKind::File
    }
    fn size(&self) -> u64 {
        0
    }
    fn read_at(&self, _off: u64, _buf: &mut [u8]) -> Result<usize, FsError> {
        Ok(0)
    }
    fn write_at(&self, _off: u64, buf: &[u8]) -> Result<usize, FsError> {
        let mut serial = unsafe { crate::drivers::serial::SerialPort::new(0x3F8) };
        for &b in buf {
            let out = if (0x20..=0x7E).contains(&b) || b == b'\n' {
                b
            } else {
                b'.'
            };
            serial.write_byte(out);
        }
        Ok(buf.len())
    }
    fn lookup(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotDir)
    }
    fn readdir(&self) -> Result<Vec<(String, InodeKind)>, FsError> {
        Err(FsError::NotDir)
    }
    fn create(&self, _name: &str, _kind: InodeKind) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotDir)
    }
    fn unlink(&self, _name: &str) -> Result<(), FsError> {
        Err(FsError::NotDir)
    }
}
