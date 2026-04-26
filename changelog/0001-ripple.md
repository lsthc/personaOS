---
build: 1
codename: Ripple
version: 0.0.2
date: 2026-04-26
summary: Boot path is sound again — fixes the HHDM-before-CR3 fault, a nightly build break, and several correctness landmines.
tags: [bootloader, kernel, shared, tooling]
---

## Highlights

- Bootloader no longer page-faults on the last write before jumping to the
  kernel.
- Builds on current nightly (the stabilized `abi_efiapi` feature gate is
  gone).
- Panic handler compiles against the modern `PanicInfo::message` API.
- `_start` is now actually kept by the linker under `--gc-sections`.
- Memory-map translation no longer overruns its buffer or mis-classifies
  the bootloader's own pages as unreclaimable kernel memory.

## Changes

### bootloader

- **Fix use-after-paging write to `BootInfo` / memory map.** The old code
  wrote to `HHDM_OFFSET + phys` *before* installing the new page tables,
  but firmware's CR3 does not map the HHDM. Those writes now go through
  the identity map, and only the pointers stored in `BootInfo` use the
  HHDM virtual address the kernel will read them at.
- **Bound the UEFI memory-map translation loop.** Entries are now capped
  at `MMAP_PAGES * 4096 / size_of::<MemoryRegion>()` so a pathological
  firmware can't walk off the end of the allocated buffer.
- **Stop reporting `LOADER_CODE` / `LOADER_DATA` as `KernelAndModules`.**
  That region is where the bootloader itself lives; classifying it as
  kernel image meant the kernel could never reclaim bootloader memory.
  It is now `BootloaderReclaimable`.
- **Remove `#![feature(abi_efiapi)]`.** The feature was stabilized; keeping
  the gate broke the build on current nightly.
- **Drop bogus `CR0` / `CR4` writes in `build_page_tables`.** Long mode is
  already on; resetting `PROTECTED_MODE_ENABLE | PAGING` mid-function is
  a footgun. Only `EFER.NXE` is still toggled, because we rely on the
  NX bit on HHDM / non-`.text` mappings.
- **Strip `USER_ACCESSIBLE` from intermediate page-table entries.** Leaf
  pages decide accessibility; granting U on PDPT/PD entries would later
  leak kernel tables to ring 3.
- **Remove unused imports** (`SimpleFileSystem`, `slice`, `Cr0/Cr4/Size4KiB`).

### kernel

- **Put `_start` in `.text._start`.** The linker script `KEEP`s that
  section; without the explicit `#[link_section]`, `--gc-sections` could
  drop the entry point.
- **Fix panic handler for current `PanicInfo` API.** Replaced the manual
  field-by-field formatting with a single `write!(serial, "{info}")`,
  which is forward-compatible with upcoming rustc changes.
- **Remove dead `BOOTED` atomic** and unused `log` dependency.
- **Clean up `font.rs`.** Replaced the `_UNUSED: [[u8; 16]; 1]` trick with
  a proper `#[allow(dead_code)]` on `DIGIT_0`.

### libs/shared

- **Correct `BOOT_INFO_MAGIC`.** The literal value did not spell
  `"PondOSB\0"` in little-endian; it now uses
  `u64::from_le_bytes(*b"PondOSB\0")` so the constant matches its
  documented byte sequence.
- **Fix `MemoryKind` doc comments.** `AcpiReclaimable` was mis-labeled as
  "UEFI runtime services"; `AcpiNvs` had no doc at all.

### tooling

- **`tools/mkdisk.sh`**: stop parsing `sgdisk --info` with awk (output
  varies across sgdisk versions). We already pin partition 1 to LBA 2048
  with `--new=1:2048:...`, so just use that constant.
- **`tools/run-qemu.sh`**: only mark `OVMF_CODE` read-only when a separate
  `OVMF_VARS` image is provided. Combined-image firmwares
  (`/usr/share/ovmf/OVMF.fd`) need to write back NVRAM and were aborting
  on boot. Also pre-create `build/` before copying vars into it.

## Notes

No boot-protocol or `BootInfo` layout changes — `BOOT_INFO_VERSION`
stays at 1. Rebuilding both crates is sufficient.
