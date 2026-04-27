//! Per-task capability table. Shape mirrors `fs::FdTable` (`kernel/src/fs/mod.rs`).

use alloc::collections::BTreeMap;
use alloc::sync::Arc;

use super::cap::Cap;

pub type CapId = i32;

pub struct CapTable {
    slots: BTreeMap<CapId, Arc<Cap>>,
    next: CapId,
}

impl Default for CapTable {
    fn default() -> Self {
        Self::new()
    }
}

impl CapTable {
    pub const fn new() -> Self {
        Self {
            slots: BTreeMap::new(),
            next: 1,
        }
    }

    pub fn install(&mut self, cap: Arc<Cap>) -> CapId {
        let id = self.next;
        self.next += 1;
        self.slots.insert(id, cap);
        id
    }

    pub fn get(&self, id: CapId) -> Option<Arc<Cap>> {
        self.slots.get(&id).cloned()
    }

    pub fn remove(&mut self, id: CapId) -> Option<Arc<Cap>> {
        self.slots.remove(&id)
    }
}
