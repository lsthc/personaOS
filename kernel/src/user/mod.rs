//! Userspace bring-up — load `/init` from the VFS as an ELF binary and
//! spawn Spring as the first ring-3 task.
//!
//! The Spring ELF is built from the early `user/init/` crate.
//! `build.rs`-style plumbing in the top-level Makefile compiles it and drops
//! the resulting ELF at `../user/init/target/x86_64-personaos-user/release/init`;
//! the kernel embeds it at compile-time via `include_bytes!` and seeds
//! `/init` in the ramfs before spawning.

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use crate::fs::{self, InodeKind};
use crate::ipc::Cap;
use crate::mm::vmm::{AddressSpace, MapFlags};
use crate::mm::PAGE_SIZE;
use crate::sched::{spawn_user, Task, TaskId};

pub const USER_STACK_TOP: u64 = 0x7FFF_F000;
pub const USER_STACK_PAGES: usize = 4;

/// Raw bytes of the Spring ELF. Assembled at compile time.
pub static INIT_ELF: &[u8] =
    include_bytes!("../../../user/init/target/x86_64-personaos-user/release/init");

/// Ensure `/init` exists in the currently-mounted root filesystem.
/// Called once from early kernel init before spawning Spring.
pub fn seed_init_into_vfs() {
    let root = fs::lookup("/").expect("no root mounted");
    match root.lookup("init") {
        Ok(_) => {}
        Err(_) => {
            let f = root.create("init", InodeKind::File).expect("create /init");
            f.write_at(0, INIT_ELF).expect("write /init");
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnError {
    Fs,
    AddressSpace,
    Elf,
    Stack,
}

/// Read an ELF from the VFS, map it into a fresh user address space, install a
/// standard user stack, and return the runnable task object.
pub fn spawn_from_path(
    path: &str,
    initial_caps: Vec<Arc<Cap>>,
    parent_pid: TaskId,
) -> Result<Arc<Task>, SpawnError> {
    let inode = fs::lookup(path).map_err(|_| SpawnError::Fs)?;
    let size = inode.size() as usize;
    let mut bytes: Vec<u8> = vec![0; size];
    let n = inode.read_at(0, &mut bytes).map_err(|_| SpawnError::Fs)?;
    bytes.truncate(n);

    let address_space = AddressSpace::new_user().ok_or(SpawnError::AddressSpace)?;
    let entry =
        unsafe { crate::fs::elf::load(&bytes, &address_space) }.map_err(|_| SpawnError::Elf)?;

    let stack_base = USER_STACK_TOP - (USER_STACK_PAGES * PAGE_SIZE) as u64;
    address_space
        .map_anon(
            stack_base,
            USER_STACK_PAGES,
            MapFlags::WRITE | MapFlags::USER | MapFlags::NX,
        )
        .map_err(|_| SpawnError::Stack)?;

    Ok(spawn_user(
        address_space,
        entry,
        USER_STACK_TOP,
        initial_caps,
        parent_pid,
    ))
}

/// Read `/init` (Spring) back out of the VFS and load it as an ELF into a
/// fresh user AS.
pub fn spawn_init() -> Arc<Task> {
    // PID 1 boots with a Registrar cap (CapId 1) so it can publish named
    // services for the rest of userspace.
    spawn_from_path("/init", vec![Cap::registrar()], 0).expect("spawn /init")
}
