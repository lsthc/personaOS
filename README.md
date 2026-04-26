<div align="center">

<img src="https://raw.githubusercontent.com/lsthc/personaOS/refs/heads/main/images/IMG_0698.png" alt="personaOS" />

**Your computer, actually yours.**

A desktop operating system built around the person using it — written from scratch in Rust.

> **Yours · Private · Clear**

[Overview](#overview) · [Install](#install) · [Build from source](#build-from-source) · [Architecture](#architecture) · [Roadmap](#roadmap) · [Contributing](#contributing)

</div>

---

## Overview

personaOS is a full desktop operating system — kernel, drivers, file
system, graphics stack, and a first-party suite of apps — designed around
three ideas:

1. **Calm over noise.** The system gets out of your way. No telemetry, no
   blinking attention grabs, no persistent upsells. A single tasteful look
   that rewards long sessions of focused work.
2. **A reflection, not a funnel.** Your computer should adapt to you, not
   the other way around. personaOS treats the user's data as the user's
   data: local by default, encrypted on disk, exportable at any time.
3. **One stack, end to end.** Every layer — from the bootloader to the
   window compositor to the file manager — is part of one coherent design.
   No mystery binaries. No layer you can't read the source of.

The engineering target is "macOS, but honest": the cohesion of a polished
commercial OS with the openness of a community project.

### At a glance

| | |
|---|---|
| **Architectures** | x86_64 (aarch64 planned) |
| **Firmware** | UEFI only |
| **Kernel** | Hybrid, Rust, `no_std`, preemptive SMP scheduler |
| **Native ABI** | Custom system calls + capability IPC |
| **Compat layer** | POSIX-ish libc, enough to port most Unix apps |
| **File system** | `PondFS` — copy-on-write, checksummed, snapshotting |
| **Display server** | `pond-surface` — Wayland-inspired, native protocol |
| **UI toolkit** | `Lily` — declarative, SwiftUI-shaped |
| **Package format** | `.drop` (signed, content-addressed) |
| **License** | MIT OR Apache-2.0 |

---

## Install

> personaOS is early software. The installer exists on the roadmap, not
> yet on any USB stick. What follows describes the intended experience;
> today, the way to run personaOS is [from source](#build-from-source).

Flash the ISO to a USB stick, boot from it, and choose **Install
personaOS**. The guided installer walks through partitioning, user
account creation, and disk encryption. It runs entirely in RAM, so your
target disk remains untouched until the final confirmation step.

To try it without committing any hardware:

```
make run        # boots build/disk.img under QEMU with OVMF
```

A live image boots to the desktop in under 5 seconds on modest hardware
(2 cores, 2 GiB RAM).

---

## Design principles

personaOS is opinionated. These commitments shape every decision.

- **Privacy is a default, not a setting.** No analytics beacons. No
  per-install identifier. Crash reports are opt-in and stripped.
- **The whole stack is memory-safe.** Rust from ring 0 up. Unsafe blocks
  are local, audited, and justified in comments.
- **One tasteful look.** Light and dark themes, one typeface family
  (*Skipstone*), one motion language (*Calm*). No theme market, no widget
  chrome bazaar. Consistency is a feature.
- **Local-first apps.** Apps run sandboxed with no network access by
  default. A capability prompt — not a checkbox in a settings pane —
  gates every escalation.
- **Boring where it counts.** The kernel is boring. The file system is
  boring. Excitement belongs in the apps you run, not in the OS under
  them.

---

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  Apps  ·  Skim (files)  Depth (terminal)  Stones (settings) │
├─────────────────────────────────────────────────────────────┤
│  Desktop shell · Dock · Menu bar · Tide (launcher)          │
├─────────────────────────────────────────────────────────────┤
│  Lily (UI toolkit)  ·  Reflection (compositor)              │
├─────────────────────────────────────────────────────────────┤
│  pond-surface (display server protocol)                     │
├─────────────────────────────────────────────────────────────┤
│  Spring (init+services)  ·  netd · audiod · vfsd  ·  libc   │
├═════════════════════════════════════════════════════════════┤  syscall boundary
│  Kernel: scheduler · VMM · IPC · VFS · drivers · PondFS     │
├─────────────────────────────────────────────────────────────┤
│  personaboot (UEFI bootloader)                              │
└─────────────────────────────────────────────────────────────┘
```

### The kernel

A hybrid kernel in the XNU sense. The scheduler, virtual memory manager,
and capability-typed IPC live in ring 0; as the system matures, risky or
failure-isolable drivers and services migrate to userspace processes that
speak the same IPC ABI.

- **Scheduler.** Preemptive, per-CPU run queues with work-stealing. Two
  scheduling classes: latency-sensitive (UI, audio) and throughput.
- **Memory.** Buddy physical allocator, slab kernel heap, demand-paged
  user address spaces. Higher-half direct map (HHDM) at
  `0xFFFF_8000_0000_0000`.
- **IPC.** Ports + capabilities. Each handle is an unforgeable token with
  a typed set of rights. No ambient authority — a service that wants to
  open a file must be given a directory capability.
- **Drivers.** PCI/MSI-X, AHCI, NVMe, xHCI (USB), PS/2, VirtIO-GPU,
  Intel HDA. Graphics drivers run in userspace; block and interrupt
  hot paths remain in-kernel.

### The bootloader — `personaboot`

A UEFI application, written in Rust. On power-on, firmware loads
`personaboot` from `\EFI\BOOT\BOOTX64.EFI`. `personaboot`:

1. Picks the largest RGB/BGR graphics mode from GOP.
2. Loads `kernel.elf` from `\EFI\personaOS\kernel.elf`.
3. Builds a fresh 4-level page table: identity map for firmware residue,
   HHDM for all of physical memory, and per-segment W^X mappings for the
   kernel image.
4. Locates the ACPI 2.0 RSDP in the UEFI configuration table.
5. Calls `ExitBootServices`, translates the UEFI memory map into our
   own format, hands a `BootInfo` to the kernel, and jumps.

See [`docs/design/boot-protocol.md`](docs/design/boot-protocol.md) for the
full ABI.

### The file system — `PondFS`

Copy-on-write, checksummed, snapshotting. Every write lands on fresh
blocks and commits atomically; a power loss rolls back to the last
committed transaction. Snapshots are O(1) and read-only by default.
Full-disk encryption with per-user keys wrapped by a TPM-sealed master
key.

### The display server — `pond-surface`

A narrow, Wayland-inspired protocol. Clients submit buffers; the
compositor (*Reflection*) composites them onto the screen with the GPU.
Every surface has a role (desktop, toplevel, popup, cursor), and the
compositor is the only code that touches the framebuffer. Input events
flow the other direction through the same socket.

### The UI toolkit — `Lily`

Declarative, shaped like SwiftUI. A `View` is a value type that describes
what should be on screen; the toolkit computes the minimal set of draw
commands needed to reconcile the previous frame with the new one.

```rust
use lily::prelude::*;

struct Counter { value: i32 }

impl View for Counter {
    fn body(&self) -> impl View {
        VStack::new((
            Text::new(format!("{}", self.value)).font(.largeTitle),
            HStack::new((
                Button::new("−").on_tap(|s: &mut Self| s.value -= 1),
                Button::new("+").on_tap(|s: &mut Self| s.value += 1),
            )),
        ))
        .padding(16)
    }
}
```

### First-party apps

| App | Purpose |
|---|---|
| **Skim** | File manager. Column view, tags, quick preview. |
| **Depth** | Terminal emulator. Ligature-aware, GPU-accelerated. |
| **Stones** | System settings. Every toggle has a plain-English explanation. |
| **Tide** | Spotlight-style launcher and universal search. |
| **Drift** | Text editor. Fast, no plugin marketplace, good defaults. |
| **Spring** (service) | Init and service manager. PID 1, supervises the rest. |
| **Stream** | Package manager UI over `.drop` archives. |

---

## Project layout

```
/
├── bootloader/                personaboot — UEFI bootloader (Rust)
├── kernel/                    The kernel (no_std, custom target)
│   ├── src/
│   │   ├── arch/x86_64/       GDT/IDT/paging/APIC/SMP
│   │   ├── mm/                Physical allocator, VMM, heap
│   │   ├── sched/             Scheduler, tasks, context switch
│   │   ├── ipc/               Ports, capabilities, shared memory
│   │   ├── fs/                VFS and PondFS
│   │   ├── drivers/           PCI, block, USB, input, framebuffer, serial
│   │   ├── net/               TCP/IP stack
│   │   └── syscall/           System-call dispatch
│   ├── kernel.ld              Linker script (−2 GiB kernel base)
│   └── x86_64-personaos-kernel.json
├── libs/
│   ├── shared/                BootInfo + types shared across the boot boundary
│   ├── libc/                  POSIX-compat libc (Rust)
│   └── libpersona/            System SDK (IPC, Lily bindings)
├── services/
│   ├── init/                  Spring — PID 1
│   ├── compositor/            Reflection
│   ├── window-server/         pond-surface
│   └── {netd, audiod, vfsd}/
├── apps/
│   └── {skim, depth, stones, tide, drift, installer}/
├── system/
│   ├── fonts/                 Skipstone
│   ├── themes/                Calm design tokens
│   └── wallpapers/
├── tools/
│   ├── bootstrap-host.sh      One-time host setup
│   ├── mkdisk.sh              Build GPT+ESP disk image
│   └── run-qemu.sh            Boot in QEMU with OVMF
├── docs/
│   ├── design/                Architecture, boot protocol, ABI
│   └── abi/                   Syscall + IPC reference
├── Cargo.toml                 Workspace
├── Makefile                   build / disk / run / debug / check / clippy
└── rust-toolchain.toml        Pinned nightly
```

---

## Build from source

### Prerequisites

One-time host bootstrap — installs Rust nightly (pinned in
`rust-toolchain.toml`), QEMU, OVMF, mtools, and gdisk:

```
./tools/bootstrap-host.sh
```

Manual install, if you prefer:

- `rustup` with the nightly toolchain listed in `rust-toolchain.toml`,
  plus the `rust-src` component and the `x86_64-unknown-uefi` target
- `qemu-system-x86_64` ≥ 8.0
- OVMF (Open Virtual Machine Firmware) — Debian/Ubuntu ship it as the
  `ovmf` package
- `mtools`, `sgdisk` (from `gdisk`), `parted`

### Common commands

```
make build       # build bootloader + kernel
make disk        # assemble build/disk.img (GPT + ESP + personaboot + kernel)
make run         # boot in QEMU with OVMF, serial to stdio
make debug       # boot paused; attach with: gdb -ex 'target remote :1234' kernel.elf
make check       # cargo check both crates
make clippy      # lints (warnings are errors)
make fmt         # rustfmt all crates
make clean       # wipe target/ and build/
```

The first build fetches crates and compiles `core`, `alloc`, and
`compiler_builtins` for the custom kernel target. Allow a few minutes.

### What "success" looks like at M0

```
$ make run
[personaboot] hello, world
[personaboot] framebuffer 1280x800 pitch=5120 bpp=32
[personaboot] loaded kernel.elf: 128744 bytes
[personaboot] kernel entry = 0xffffffff80001000
[personaboot] kernel stack top = 0xffff800000101000
[personaboot] rsdp = 0x7fbe014
[personaboot] pml4 @ 0x7f8a000
[kernel] _start reached
[kernel] BootInfo OK
[kernel] M0 milestone reached, halting.
```

The QEMU window shows a dark-navy background with two lines of lavender
text: **personaOS booted** and **M0 — kernel entry reached, framebuffer
online**.

---

## Roadmap

personaOS is built in six milestones. We are currently at **M0**.

| Milestone | Scope | Status |
|---|---|---|
| **M0 — Hello, kernel** | `personaboot` + kernel entry + serial + framebuffer | **in progress** |
| M1 — Core kernel | GDT/IDT, APIC, paging, heap, preemptive scheduler, ring-3, ELF loader, first user process | planned |
| M2 — Storage & devices | PCI, AHCI/NVMe, xHCI, VFS, PondFS, GPT, PS/2 + USB input | planned |
| M3 — Userspace | Spring (init), capability IPC, terminal, Shore (shell), users & permissions | planned |
| M4 — Graphics | VirtIO-GPU driver, pond-surface, Reflection compositor, Skipstone fonts, Lily toolkit | planned |
| M5 — Desktop | Dock, menu bar, Tide launcher, Skim/Depth/Stones/Drift, networking, audio | planned |
| M6 — Installer & packages | Live ISO, guided installer, `.drop` format, Stream package manager | planned |
| M7+ | Additional hardware, App sandboxing, aarch64 port (Apple Silicon, Raspberry Pi) | future |

The realistic timeline to a usable desktop is measured in years, not
weekends. Progress is tracked milestone by milestone; nothing is shipped
until it works end-to-end.

---

## Contributing

personaOS is MIT/Apache-2.0 dual-licensed and accepts external
contributions. The guiding rules:

- **Every PR ships with tests or a reasoned explanation of why not.**
- **No third-party code paths without an audit note.** The kernel depends
  on a small vetted list of crates that only provide *types* — everything
  else is written here.
- **Match the tone.** The system never scolds the user. Neither should
  the error messages or the docs.
- **Read the design doc before the patch.** `docs/design/` is the source
  of truth; if a design decision is wrong, fix the document first.

File issues at the project tracker. New contributors are welcome — the
*good first issue* label marks self-contained tasks that do not require
deep kernel knowledge.

---

## Names & conventions

personaOS names its own pieces. The theme is water — still, reflective,
clear — as a reminder of the values above.

| Piece | Name | Meaning |
|---|---|---|
| Bootloader | `personaboot` | First light |
| File system | `PondFS` | Self-contained, complete |
| IPC ABI | `Channel` | A path for water to flow |
| Display protocol | `pond-surface` | Where every pixel meets |
| Compositor | `Reflection` | The image on the surface |
| UI toolkit | `Lily` | Floats on the surface, shows the beauty |
| Shell | `Shore` | Edge between user and kernel |
| Init / service manager | `Spring` | The source that feeds the rest |
| Package format | `.drop` | A single, self-contained app |
| Package manager | `Stream` | How drops arrive |
| App store | `Tide` | Regular, predictable arrivals |
| File manager | `Skim` | A glance across the surface |
| Terminal | `Depth` | What lies beneath |
| Settings | `Stones` | Small placements, visible ripples |
| Text editor | `Drift` | Calm, unhurried authorship |
| System font | `Skipstone` | Light, accurate, travels far |
| Design language | `Calm` | Motion, color, spacing |

---

## License

```
Copyright © 2026 the personaOS contributors.

Licensed under either of
  * Apache License, Version 2.0
  * MIT License
at your option.
```

See [`LICENSE-APACHE`](LICENSE-APACHE) and [`LICENSE-MIT`](LICENSE-MIT)
for the full texts.
