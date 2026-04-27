//! Named-service registry.
//!
//! A global map from `String` to a raw `Arc<Port>`. Publishers need a cap with
//! `CapObject::Registrar`; lookups are unrestricted and return a fresh
//! SEND-only cap. Spring (PID 1) owns the registrar cap and mediates service
//! startup.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;

use spin::Mutex;

use super::port::Port;

static REGISTRY: Mutex<BTreeMap<String, Arc<Port>>> = Mutex::new(BTreeMap::new());

pub fn publish(name: &str, port: Arc<Port>) -> Result<(), Errno> {
    let mut r = REGISTRY.lock();
    if r.contains_key(name) {
        return Err(Errno::Exists);
    }
    r.insert(String::from(name), port);
    Ok(())
}

pub fn lookup(name: &str) -> Option<Arc<Port>> {
    REGISTRY.lock().get(name).cloned()
}

#[derive(Clone, Copy)]
pub enum Errno {
    Exists,
}
