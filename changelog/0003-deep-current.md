---
build: 3
codename: Deep Current
version: 0.1.0
date: 2026-04-26
summary: M2 pipeline lands — PCI enumeration, VFS, NVMe block driver, PondFS (RW), real ELF init, and an expanded syscall surface.
tags: [kernel, drivers, fs, userspace, tooling]
---

## Highlights

- **PCI(e) enumeration over ACPI MCFG / ECAM.** Every function on every bus
  is probed, BARs sized, and capability chains walked. Output looks a lot
  like `lspci` on the serial console.
- **Virtual filesystem + ramfs + FD tables.** Tasks now have open-file
  tables; syscall numbers 4..=8 cover `open` / `read` / `close` / `lseek`
  / `fstat`. Ramfs is mounted at `/boot` as scratch.
- **NVMe 1.x driver.** Brings up QEMU's `qemu-nvme` controller, runs
  Identify Controller / Active NSID / Identify Namespace, creates an I/O
  queue pair, and exposes a `BlockDevice` backed by polled PRP1 transfers.
- **PondFS lands read-write.** A tiny inode + bitmap + direct-block
  filesystem, formatted from the host with `mkpondfs`, mounted on `/`
  from NVMe on boot. Create, read, write, and unlink all work; `init`
  round-trips a scratch file at boot time.
- **Real ELF init.** The `user/init` crate is now the userspace program;
  the kernel embeds its ELF at build time, seeds it into the filesystem,
  and loads it into ring 3 via a proper PT_LOAD loader instead of the
  hand-rolled blob we shipped in M1.
- **Syscall ABI aligned with Linux.** `syscall_entry` now preserves every
  register the 64-bit SysV syscall convention says the kernel leaves
  untouched (`rdi`/`rsi`/`rdx`/`r10`/`r8`/`r9` plus all callee-saved),
  which fixed a stealth bug where the second syscall in a sequence saw
  garbage args.

## Changes

### kernel / arch

- **`arch::x86_64::acpi`** — new module. Parses RSDP → XSDT → MCFG and
  exposes the ECAM allocation list. ACPI 2.0+ only; we never touched the
  legacy RSDT path.
- **`arch::x86_64::idt`** — the IDT is now a `UnsafeCell` wrapped in a
  `Once`, with a `Mutex`-guarded vector-allocation bitmap for MSI/MSI-X.
  `alloc_vector(handler)` hands out free vectors in `0x30..=0xEF`.
- **`arch::x86_64::syscall`** — `syscall_entry` saves and restores
  `rdi/rsi/rdx/r10/r8/r9` across the dispatcher call. Without this,
  userspace that issues two syscalls back-to-back without reloading its
  arg registers (which is what `rustc` emits at `-O`) silently corrupted
  every call after the first.

### kernel / mm

- **`mm::vmm::MapFlags::MMIO`** — sets PCD|PWT on leaf entries for UC
  mappings. New `map_mmio(phys, bytes)` returns a kernel virtual address
  out of a dedicated UC window at `0xFFFF_9000_0000_0000`, so device
  BARs don't collide with the HHDM's WB cacheability.

### kernel / sched

- **`TaskState::Blocked`** plus a `WAIT_QUEUES` keyed by `usize`. New
  `block_on(key)` parks the current task; `wake_all(key)` moves every
  blocked waiter back onto the run queue. IRQ-safe, alloc-free on the
  wake path.
- **`Task::fds: Mutex<FdTable>`.** Per-task open-file tables. `spawn_user`
  pre-installs fd 0/1/2 pointing at a `SerialStdout` shim so existing
  stdio-style writes keep working.

### kernel / drivers

- **`drivers::pci`** (new) — ECAM reader, BAR sizer, MSI-X capability
  parser, device registry, `find_class(class, subclass, prog_if)`.
- **`drivers::block`** (new) — `BlockDevice` trait and a global registry.
- **`drivers::nvme`** (new) — full bring-up to polled I/O on one
  controller / one namespace. PRP1-only, 4 KiB cap per transfer, no MSI-X
  yet — the scheduler already has the infrastructure when we want it.

### kernel / fs

- **`fs`** (new) — Inode/Filesystem traits, mount table with longest-prefix
  match, FD table, `SerialStdout` shim.
- **`fs::ramfs`** — in-memory filesystem backing `/boot`.
- **`fs::elf`** — `xmas-elf`-based loader for static 64-bit ELFs. Parses
  PT_LOADs, maps them into a user `AddressSpace` with proper R/W/X, and
  copies bytes in with a brief CR3 switch. Returns the entry point.
- **`fs::pondfs`** — superblock + bitmap + inode-table + 12 direct
  blocks. RW; supports create / read / write / unlink / readdir.

### kernel / syscall

- Numbering: `0 write`, `1 exit`, `2 yield`, `3 getpid`, `4 open`,
  `5 read`, `6 close`, `7 lseek`, `8 fstat`.

### userspace

- **`user/init/` crate** — a standalone `#![no_std]` binary built for a
  new `x86_64-personaos-user` target. Links at `0x0040_0000` via its own
  linker script. The kernel embeds its release ELF at compile time.
- init now prints `[init] hello from userspace`, prints its pid, opens
  and reads `/hello.txt`, yields three times, writes and re-reads
  `/scratch.txt` to demo PondFS RW, and exits.

### tooling

- **`tools/mkpondfs/`** (new, independent host crate) — formats a raw
  image with a PondFS superblock, bitmap, inode table, and a seeded file
  tree. `tools/mkdisk.sh` builds a 32 MiB PondFS image, copies the init
  ELF to `/init`, and writes a `/hello.txt` greeting.
- **`tools/run-qemu.sh`** — attaches the PondFS image as an NVMe namespace
  via `-device nvme,drive=pond` when `build/pond.img` is present.
- **`Makefile`** — gained a `build-init` target that is now a dependency
  of `build`, so the kernel always sees an up-to-date init ELF.

## Notes

- Boot ABI is unchanged; `BOOT_INFO_VERSION` stays at 1.
- xHCI / USB is **not** in this build. It is the only M2 goal deferred;
  it will land in a follow-up build together with input support.
- PondFS files are capped at 12 direct blocks × 4 KiB = 48 KiB. The init
  ELF (~11 KiB) fits comfortably. Single-indirect blocks are the first
  obvious extension.
- No journaling. A crash mid-write may leave a dangling bitmap bit or a
  partially-written directory entry. Acceptable for now; add `fsck`-style
  recovery when userspace grows real workloads.
- The admin and I/O NVMe queues poll completions on the submitting CPU.
  A follow-up will allocate an MSI-X vector via `idt::alloc_vector` and
  flip completion waits to `sched::block_on`.
