---
build: 5
codename: Reflection Pool
version: 0.3.0
date: 2026-04-27
summary: M4 graphics smoke lands — framebuffer display syscalls, Reflection, pond-surface IPC, Lily surface helper, Skipstone text metadata, and VirtIO-GPU discovery/fallback.
tags: [kernel, graphics, ipc, userspace, compositor, lily]
---

## Highlights

- **Display syscalls.** The boot framebuffer is now a synchronized kernel display service with userspace calls for display info, clear, rectangle fills, and bitmap text.
- **Reflection compositor smoke.** Spring launches Reflection from PondFS at `/bin/reflection`; Reflection draws the first compositor panel and owns a surface IPC port.
- **pond-surface seed.** Reflection passes a surface send capability to Spring, Spring publishes `com.persona.reflection`, and `surface-demo` looks it up to submit draw and shutdown requests.
- **Lily starts small.** `libs/lily` provides a tiny declarative `SurfaceCard` helper that composes the first surface request instead of hand-rolling registers in the demo app.
- **Skipstone metadata.** `libs/skipstone` defines the first text style IDs and metrics; Lily sends style metadata and Reflection renders/logs the selected style.
- **VirtIO-GPU discovery.** The kernel now detects whether a VirtIO-GPU candidate is present and explicitly reports framebuffer fallback when it is not.

## Changes

### kernel / graphics

- `drivers::framebuffer` now owns a global synchronized display instance, exposes `DisplayInfo`, and supports clear, fill rect, and byte-string text drawing.
- `syscall` numbers 18–21 expose `display_info`, `display_clear`, `display_fill_rect`, and `display_draw_text` to userspace.
- `drivers::virtio_gpu` performs minimal PCI discovery for VirtIO-GPU candidates and reports whether the framebuffer backend remains active.
- Kernel boot now initializes the framebuffer service before userspace and logs the active graphics backend.

### userspace / compositor

- Added **Reflection** (`user/reflection`) as the first compositor-shaped service.
- Reflection creates a surface port, sends a duplicate send cap to Spring, serves `SURFACE_DRAW` and `SURFACE_SHUTDOWN`, and renders client surfaces through display syscalls.
- Added **surface-demo** (`user/surface-demo`) as the first pond-surface client.
- Spring now starts Reflection, publishes `com.persona.reflection`, starts surface-demo, waits for both, then continues into Depth/Shore.

### libraries

- `libpersona` now exposes display syscall wrappers and shared graphics IPC opcodes.
- Added `libs/lily` with the first `SurfaceCard` helper.
- Added `libs/skipstone` with minimal text style metadata for M4.

### tooling

- `Makefile` builds Reflection and surface-demo as part of `make build`.
- `tools/mkdisk.sh` seeds `/bin/reflection`, `/bin/surface-demo`, and their Spring manifest entries into PondFS.

## Verification

- `make build` succeeds for userspace crates, bootloader, and kernel.
- `make fmt` succeeds.
- `make clippy` succeeds for bootloader and kernel with warnings denied.
- `make disk` seeds the full M4 userland into PondFS.
- QEMU boot demonstrates:
  - framebuffer backend selection and VirtIO-GPU fallback logging.
  - Spring loading the manifest and supervising vfsd, Reflection, surface-demo, Depth, and Shore.
  - Reflection receiving a surface draw request over IPC.
  - surface-demo sending draw and shutdown through the published Reflection service.
  - Reflection rendering a Lily/Skipstone-styled surface and exiting cleanly.
  - Depth/Shore still reaching the interactive shell prompt.

## Notes

- Reflection still draws through framebuffer syscalls; there is no GPU command queue yet.
- pond-surface is still an inline-register protocol smoke, not a buffer protocol.
- Lily and Skipstone are metadata/helper crates for now; real layout, shaping, and font rasterization remain M5+ work.
