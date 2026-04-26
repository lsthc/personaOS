//! In-memory filesystem. Used before block storage comes online and as
//! `/boot` once PondFS takes over `/`.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use spin::Mutex;

use super::{FsError, Filesystem, Inode, InodeKind};

struct Entry {
    kind: InodeKind,
    data: Vec<u8>,
    children: BTreeMap<String, Arc<RamInode>>,
}

pub struct RamInode {
    inner: Mutex<Entry>,
}

impl RamInode {
    fn new_file(data: Vec<u8>) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Entry {
                kind: InodeKind::File,
                data,
                children: BTreeMap::new(),
            }),
        })
    }

    fn new_dir() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Entry {
                kind: InodeKind::Dir,
                data: Vec::new(),
                children: BTreeMap::new(),
            }),
        })
    }
}

impl Inode for RamInode {
    fn kind(&self) -> InodeKind {
        self.inner.lock().kind
    }

    fn size(&self) -> u64 {
        self.inner.lock().data.len() as u64
    }

    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        let g = self.inner.lock();
        if g.kind != InodeKind::File {
            return Err(FsError::IsDir);
        }
        let off = off as usize;
        if off >= g.data.len() {
            return Ok(0);
        }
        let n = core::cmp::min(buf.len(), g.data.len() - off);
        buf[..n].copy_from_slice(&g.data[off..off + n]);
        Ok(n)
    }

    fn write_at(&self, off: u64, buf: &[u8]) -> Result<usize, FsError> {
        let mut g = self.inner.lock();
        if g.kind != InodeKind::File {
            return Err(FsError::IsDir);
        }
        let off = off as usize;
        let end = off + buf.len();
        if end > g.data.len() {
            g.data.resize(end, 0);
        }
        g.data[off..end].copy_from_slice(buf);
        Ok(buf.len())
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> {
        let g = self.inner.lock();
        if g.kind != InodeKind::Dir {
            return Err(FsError::NotDir);
        }
        g.children.get(name).cloned().map(|c| c as Arc<dyn Inode>).ok_or(FsError::NotFound)
    }

    fn readdir(&self) -> Result<Vec<(String, InodeKind)>, FsError> {
        let g = self.inner.lock();
        if g.kind != InodeKind::Dir {
            return Err(FsError::NotDir);
        }
        Ok(g.children.iter().map(|(n, c)| (n.clone(), c.inner.lock().kind)).collect())
    }

    fn create(&self, name: &str, kind: InodeKind) -> Result<Arc<dyn Inode>, FsError> {
        let mut g = self.inner.lock();
        if g.kind != InodeKind::Dir {
            return Err(FsError::NotDir);
        }
        if g.children.contains_key(name) {
            return Err(FsError::Exists);
        }
        let node = match kind {
            InodeKind::File => RamInode::new_file(Vec::new()),
            InodeKind::Dir => RamInode::new_dir(),
        };
        g.children.insert(String::from(name), node.clone());
        Ok(node)
    }

    fn unlink(&self, name: &str) -> Result<(), FsError> {
        let mut g = self.inner.lock();
        if g.kind != InodeKind::Dir {
            return Err(FsError::NotDir);
        }
        g.children.remove(name).ok_or(FsError::NotFound)?;
        Ok(())
    }
}

pub struct RamFs {
    root: Arc<RamInode>,
}

impl RamFs {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { root: RamInode::new_dir() })
    }

    /// Seed a file at absolute path `path`, creating any missing directories
    /// along the way. Panics on malformed paths (caller is the kernel itself).
    pub fn put_file(&self, path: &str, bytes: &[u8]) {
        assert!(path.starts_with('/'));
        let mut node: Arc<dyn Inode> = self.root.clone();
        let comps: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
        let (last, dirs) = comps.split_last().expect("non-empty");
        for d in dirs {
            node = match node.lookup(d) {
                Ok(n) => n,
                Err(_) => node.create(d, InodeKind::Dir).expect("ramfs: create dir"),
            };
        }
        let file = match node.lookup(last) {
            Ok(f) => f,
            Err(_) => node.create(last, InodeKind::File).expect("ramfs: create file"),
        };
        file.write_at(0, bytes).expect("ramfs: seed write");
    }
}

impl Filesystem for RamFs {
    fn root(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }
}
