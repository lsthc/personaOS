//! Block device abstraction shared by all storage drivers.

use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum BlockError {
    OutOfRange,
    BadLength,
    Io,
    Unsupported,
}

#[allow(dead_code)] // write_blocks / block_count read by pondfs as it lands
pub trait BlockDevice: Send + Sync {
    fn block_size(&self) -> usize;
    fn block_count(&self) -> u64;
    fn read_blocks(&self, lba: u64, buf: &mut [u8]) -> Result<(), BlockError>;
    fn write_blocks(&self, lba: u64, buf: &[u8]) -> Result<(), BlockError>;
}

static DEVICES: Mutex<Vec<Arc<dyn BlockDevice>>> = Mutex::new(Vec::new());

pub fn register(dev: Arc<dyn BlockDevice>) {
    DEVICES.lock().push(dev);
}

/// First registered device — the root filesystem lives here.
pub fn first() -> Option<Arc<dyn BlockDevice>> {
    DEVICES.lock().first().cloned()
}
