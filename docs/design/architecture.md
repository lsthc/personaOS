# personaOS — Architecture Overview (M0)

> Status: living document. The scope below describes the long-term target;
> only the **boot** and **kernel entry** layers exist as of M0.

## Goals

1. A complete desktop operating system that a user can install to disk and
   use as a daily driver.
2. macOS-inspired UX: a single tasteful look, consistent typography, smooth
   animations, a coherent SDK, and a small set of first-party apps.
3. Memory-safe kernel and userspace written almost entirely in Rust.
4. x86_64 first; aarch64 when the stack is stable enough to port.

## Non-goals (initial)

- POSIX certification. We aim for "macOS-level" compatibility: enough of a
  POSIX surface for ports, but our native ABI is our own.
- BIOS / legacy boot. UEFI only.
- Multi-distribution packaging. One OS, one image.

## Layering

```
┌─────────────────────────────────────────────┐
│ Apps (Finder, Terminal, Settings, ...)      │
├─────────────────────────────────────────────┤
│ Desktop shell / window manager              │
├─────────────────────────────────────────────┤
│ Compositor + graphics server (pond-wl)      │
├─────────────────────────────────────────────┤
│ System services (init, auth, netd, audiod)  │
├─────────────────────────────────────────────┤
│ libpersona (SDK) · libc (POSIX compat)      │
├═════════════════════════════════════════════┤ ← syscall boundary
│ Kernel: scheduler, VMM, IPC, VFS, drivers   │
├─────────────────────────────────────────────┤
│ personaboot (UEFI)                          │
└─────────────────────────────────────────────┘
```

## Kernel model

**Hybrid**, in the XNU sense. Start as a tidy monolithic kernel with clear
internal module boundaries; migrate high-risk, failure-isolable components
(file systems, device drivers that can afford the cost) to userspace
services once IPC is mature. This lets M1–M3 move quickly while preserving
the option to tighten security and reliability later without a rewrite.

## Boot flow (M0)

1. Firmware loads `\EFI\BOOT\BOOTX64.EFI` (our `personaboot`) from the ESP.
2. `personaboot` queries GOP for a framebuffer, reads `kernel.elf` from the
   ESP, parses it, allocates physical pages for each `PT_LOAD` segment, and
   builds fresh page tables with:
   - an identity map of the low 4 GiB (kept for firmware residue);
   - the higher-half direct map (HHDM) at `0xFFFF_8000_0000_0000`;
   - kernel segments at their linked addresses (−2 GiB region).
3. It finds the ACPI RSDP via the UEFI configuration table, calls
   `ExitBootServices`, translates the UEFI memory map to our own format,
   and fills a `BootInfo` in memory it owns.
4. It writes CR3, switches to a fresh 64 KiB kernel stack in the HHDM, and
   jumps to `_start(&BootInfo)`.
5. The kernel validates `BootInfo`, initializes the serial port, paints a
   confirmation message to the framebuffer, and halts.

See [`boot-protocol.md`](boot-protocol.md) for the exact ABI.

## Source layout

- `bootloader/` — `personaboot`, UEFI boot application (target
  `x86_64-unknown-uefi`).
- `kernel/` — the kernel, freestanding `no_std` (custom target
  `x86_64-personaos-kernel.json`, linked at `0xFFFFFFFF80000000`).
- `libs/shared/` — types shared across the boot boundary: `BootInfo`,
  `Framebuffer`, `MemoryMap`.
- `tools/` — `mkdisk.sh` (GPT + ESP image), `run-qemu.sh` (OVMF + QEMU).

## Milestones

| Milestone | Scope                                                          |
|-----------|----------------------------------------------------------------|
| **M0**    | Boot to framebuffer text via `personaboot` + stub kernel.      |
| M1        | Interrupts, paging, heap, preemptive scheduler, ring-3, ELF.   |
| M2        | PCI, NVMe/AHCI, xHCI, VFS, `PondFS`.                           |
| M3        | init, services, IPC, terminal, shell.                          |
| M4        | VirtIO-GPU, `pond-wl`, compositor, `PersonaUI` toolkit.        |
| M5        | Desktop shell, first-party apps, networking, audio.            |
| M6        | Installer, live ISO, `.pond` package manager.                  |

## Conventions

- Unsafe blocks are local and annotated. `forbid(unsafe_op_in_unsafe_fn)`
  is the default.
- No allocator in the kernel until M1 introduces it; M0 is purely static.
- Arch-specific code lives under `kernel/src/arch/<arch>/`. The rest of
  the kernel must compile on any supported architecture.
