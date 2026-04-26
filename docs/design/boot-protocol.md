# personaOS Boot Protocol

The ABI between `personaboot` (UEFI bootloader) and the kernel.

## Entry

The kernel exports the symbol `_start` with the AMD64 SysV calling
convention:

```rust
pub unsafe extern "sysv64" fn _start(info: *const BootInfo) -> !;
```

When `_start` runs:

- **RDI** holds a pointer to a valid `BootInfo` in kernel-readable memory.
- **RSP** points to the top of a 64 KiB stack in the HHDM. **RBP** is zero.
- CR3 already points at the kernel's final top-level page table.
- Paging is on. Long mode is on. NXE is enabled.
- UEFI boot services have been exited; the UART at `0x3F8` is safe to use
  and no other I/O is yet initialized by the kernel.
- Interrupts are disabled.

## `BootInfo`

Declared in `persona-shared` and stable across the boundary:

```rust
#[repr(C)]
pub struct BootInfo {
    magic:        u64,            // BOOT_INFO_MAGIC = "PondOSB\0"
    version:      u32,            // BOOT_INFO_VERSION, currently 1
    _pad0:        u32,
    framebuffer:  Framebuffer,
    memory_map:   MemoryMap,
    rsdp_phys:    u64,            // ACPI 2.0 RSDP physical address, or 0
    cmdline_ptr:  *const u8,
    cmdline_len:  usize,
    hhdm_offset:  u64,             // currently 0xFFFF_8000_0000_0000
}
```

The kernel must check `magic == BOOT_INFO_MAGIC` and panic on mismatch.
`version` permits additive changes; the kernel should treat higher
versions as an error.

## Address space

- **Higher-half direct map** at `hhdm_offset`: `phys + hhdm_offset == virt`
  for any physical address within the mapped range. Bootloader guarantees
  at least the first 4 GiB are mapped, plus any framebuffer window the
  firmware exposes above that.
- **Kernel image** is linked at `0xFFFFFFFF80000000` and is mapped using
  4 KiB pages with per-segment permissions (W^X respected).
- **Low identity map** (0 .. 4 GiB) is mapped writable+executable. The
  kernel is expected to tear this down in M1 once it has taken over.

## Memory map

`BootInfo.memory_map.entries` points to an array of `MemoryRegion` living
in `BootloaderReclaimable` memory. The kernel should copy it into its own
storage before reclaiming that region.

`MemoryKind::Usable` regions are free for the kernel's physical frame
allocator. `BootloaderReclaimable` is additionally free once the kernel
no longer needs the `BootInfo` data it references.

## Stability

Version bumps are breaking changes to the layout above. Any additions
should append new fields (and bump the version), never reorder existing
ones.
