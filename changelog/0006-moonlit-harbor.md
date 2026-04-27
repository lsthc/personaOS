---
build: 6
codename: Moonlit Harbor
version: 0.4.0
date: 2026-04-27
summary: M5 desktop lands — macOS-style shell smokes, first apps, Reflection keyboard input, Depth, e1000 DHCP, and AC97 PCM playback.
tags: [desktop, networking, audio, userspace, drivers]
---

## Highlights

- **Desktop shell complete.** Reflection now draws the first macOS-style desktop shell with menu bar, Dock, and status items fed by real service state.
- **First-party app smokes.** Tide, Skim, Stones, and Drift submit pond-surface requests and render through the compositor path.
- **Keyboard input reaches the compositor.** Reflection consumes PS/2-backed keyboard events without leaking the compositor smoke key into Shore.
- **Real networking.** The kernel drives QEMU's e1000 NIC, transmits a DHCPDISCOVER, receives a DHCPOFFER, and reports the offered `10.0.2.15` configuration through `netd`.
- **Real audio.** The kernel drives QEMU's AC97 controller and plays a short PCM tone through the WAV backend at `build/audio.wav`.

## Changes

### kernel / drivers

- Added an e1000 PCI/MMIO driver with polling TX/RX descriptor rings and bounded DHCP offer parsing.
- Added an AC97 PCI/PIO driver with bus-master PCM-out playback.
- Added 32-bit PIO helpers and PCI I/O-space bus-master enablement for legacy audio hardware.
- Added syscalls for network configuration/status and audio playback/status.

### userspace / services

- Added **netd** as a Spring-supervised service that invokes the real e1000 DHCP path before reporting readiness.
- Added **audiod** as a Spring-supervised service that invokes real AC97 PCM playback before reporting readiness.
- Spring now loads 11 manifest services and starts netd/audiod before the desktop shell.
- The desktop shell derives Wi-Fi/Sound status flags from kernel network/audio state instead of hard-coded success.

### tooling

- `tools/run-qemu.sh` now attaches deterministic e1000 networking and AC97 WAV audio devices.
- `tools/mkdisk.sh` seeds `/sbin/netd`, `/sbin/audiod`, and their Spring manifest entries.
- `Makefile` builds netd and audiod as part of the normal image build.

## Verification

- `make fmt` succeeds.
- `make build` succeeds for userspace crates, bootloader, and kernel.
- `make clippy` succeeds for bootloader and kernel with warnings denied.
- `make disk` seeds the full M5 userland into PondFS.
- QEMU boot demonstrates:
  - PCI discovery of e1000 (`8086:100e`) and AC97 (`8086:2415`).
  - e1000 link-up, DHCPDISCOVER TX, DHCPOFFER RX, and `netd` reporting `10.0.2.15`.
  - AC97 PCM playback completion and a non-empty `build/audio.wav` artifact.
  - Reflection desktop, Dock/menu bar, first app smokes, keyboard input, Depth/Shore `hello` and `exit`, and Spring exiting cleanly.
