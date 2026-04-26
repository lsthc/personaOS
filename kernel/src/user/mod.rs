//! Userspace bring-up — load `/init` from the VFS as an ELF binary and
//! spawn it as the first ring-3 task.
//!
//! The init ELF is built as a standalone Rust crate under `user/init/`.
//! `build.rs`-style plumbing in the top-level Makefile compiles it and drops
//! the resulting ELF at `../user/init/target/x86_64-personaos-user/release/init`;
//! the kernel embeds it at compile-time via `include_bytes!` and seeds
//! `/init` in the ramfs before spawning.

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use crate::fs::{self, InodeKind};
use crate::mm::vmm::{AddressSpace, MapFlags};
use crate::mm::PAGE_SIZE;
use crate::sched::{spawn_user, Task};

pub const USER_STACK_TOP: u64 = 0x7FFF_F000;
pub const USER_STACK_PAGES: usize = 4;

/// Raw bytes of the init ELF. Assembled at compile time.
pub static INIT_ELF: &[u8] = include_bytes!(
    "../../../user/init/target/x86_64-personaos-user/release/init"
);

/// Ensure `/init` exists in the currently-mounted root filesystem.
/// Called once from early kernel init before spawning.
pub fn seed_init_into_vfs() {
    let root = fs::lookup("/").expect("no root mounted");
    match root.lookup("init") {
        Ok(_) => {}
        Err(_) => {
            let f = root
                .create("init", InodeKind::File)
                .expect("create /init");
            f.write_at(0, INIT_ELF).expect("write /init");
        }
    }
}

/// Read `/init` back out of the VFS (exercises the block layer / filesystem
/// path end-to-end) and load it as an ELF into a fresh user AS.
pub fn spawn_init() -> Arc<Task> {
    let inode = fs::lookup("/init").expect("/init not in VFS");
    let size = inode.size() as usize;
    let mut bytes: Vec<u8> = vec![0; size];
    let n = inode.read_at(0, &mut bytes).expect("read /init");
    bytes.truncate(n);

    let address_space = AddressSpace::new_user().expect("alloc user PML4");

    let entry = unsafe {
        crate::fs::elf::load(&bytes, &address_space).expect("load /init ELF")
    };

    // Stack: 4 pages at the top of the user range.
    let stack_base = USER_STACK_TOP - (USER_STACK_PAGES * PAGE_SIZE) as u64;
    address_space
        .map_anon(
            stack_base,
            USER_STACK_PAGES,
            MapFlags::WRITE | MapFlags::USER | MapFlags::NX,
        )
        .expect("map user stack");

    spawn_user(address_space, entry, USER_STACK_TOP)
}
