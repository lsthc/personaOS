---
build: 4
codename: Spring Tide
version: 0.2.0
date: 2026-04-26
summary: M3 userspace lands — Spring, capability IPC, process lifecycle, PS/2-backed TTY, Depth/Shore, and libpersona.
tags: [kernel, ipc, sched, userspace, services, tty]
---

## Highlights

- **Spring is now PID 1.** The stable boot path still loads `/init`, but the runtime identity is Spring: it owns the registrar capability, publishes `com.persona.spring`, reads `/etc/spring.toml`, and supervises early services.
- **Capability IPC.** Ring-3 tasks can create ports, duplicate/drop capabilities, publish/lookup named services, transfer inline registers, and move page payloads with zero-copy page stealing.
- **Process lifecycle.** `spawn`, `waitpid`, and `kill` are wired through the scheduler, task registry, parent tracking, exit codes, and wait queues.
- **First spawned service.** `vfsd` is loaded from PondFS at `/sbin/vfsd`, reads `/hello.txt`, notifies Spring over IPC, exits, and is observed by Spring via `waitpid`.
- **TTY input.** A PS/2 keyboard decoder feeds fd 0 through a minimal TTY stdin shim with line input, echo, and backspace handling.
- **Depth + Shore.** Spring launches Depth at `/bin/depth`; Depth launches the Shore shell at `/bin/shore`; Shore accepts `help`, `hello`, `run-vfsd`, and `exit` over the PS/2-backed TTY.
- **libpersona.** Userspace crates now share a small `no_std` SDK for syscall numbers, IPC ABI structs, wrappers, and serial formatting helpers.

## Changes

### kernel / arch

- **`arch::x86_64::syscall`** now switches from the user stack to the current task's kernel stack before calling Rust syscall dispatch. This fixes the multi-address-space failure where `spawn`/`exit` could switch CR3 while the kernel was still executing on a user stack.

### kernel / mm

- **`AddressSpace::phys_for_virt`, `copy_to_user`, and `zero_user`** let the kernel write into a target userspace address space through HHDM-backed physical mappings.
- **`fs::elf`** uses those helpers instead of switching CR3 during ELF load, so `sys_spawn` can safely load child processes from syscall context.
- **IPC VA window** at `0x0000_2000_0000..0x0000_6000_0000` receives page-stolen IPC payloads.

### kernel / ipc

- New `kernel/src/ipc/` module with:
  - `cap` — `Cap`, `CapObject`, and `Rights` for ports, registrar, and future VM objects.
  - `cap_table` — per-task capability tables.
  - `port` — bounded message queues carrying six inline registers, transferred caps, and stolen pages.
  - `registry` — global named-service registry.
  - `syscalls` — syscall implementations for port create, send/recv, cap drop/dup, and name publish/lookup.
- PID 1 receives the registrar capability at spawn time.
- Page payload sends detach sender pages and install them into the receiver's IPC window.

### kernel / sched

- Tasks now track parent PID and exit code.
- Global task registry supports child lookup, exited-child discovery, and kill-by-pid.
- `current_exit` marks the task dead, wakes the parent wait queue, and never returns to the exiting userspace frame.
- `sys_waitpid` blocks on a parent-specific wait key until a matching child exits.

### kernel / drivers + fs

- New **PS/2 keyboard driver** decodes set-1 scancodes to ASCII, including shift/caps state and basic punctuation.
- fd 0 for new userspace tasks now points at `TtyStdin`; fd 1/2 still point at serial stdout.
- `TtyStdin` implements blocking line input over PS/2, with echo and backspace.

### userspace

- **Spring (`user/init`)** publishes `com.persona.spring`, runs IPC self-tests, starts `vfsd`, starts Depth, and waits for supervised children.
- **vfsd (`user/vfsd`)** proves service lifecycle and service-to-Spring IPC.
- **Depth (`user/depth`)** is the first terminal host; it starts Shore and reports the shell exit status.
- **Shore (`user/shore`)** is the first shell prompt. It can print help, read `/hello.txt`, spawn `/sbin/vfsd`, and exit.
- **libpersona (`libs/libpersona`)** is the first shared userspace SDK crate.

### tooling

- `Makefile` builds init, vfsd, Shore, Depth, bootloader, and kernel as part of `make build`.
- `tools/mkdisk.sh` seeds `/sbin/vfsd`, `/bin/depth`, `/bin/shore`, and a Spring TOML manifest into PondFS.

## Verification

- `make build` succeeds for all userspace crates, bootloader, and kernel.
- `make clippy` succeeds for bootloader and kernel with warnings denied.
- QEMU boot demonstrates:
  - Spring loading `/etc/spring.toml`.
  - vfsd starting, reading `/hello.txt`, notifying Spring, and exiting cleanly.
  - Depth starting Shore.
  - Shore reading `hello` over PS/2-backed TTY input, printing `hello from pond`, then exiting.
  - Depth and Spring observing clean exit statuses.

## Notes

- `vfsd` is still a lifecycle smoke, not the real file broker. Kernel VFS remains authoritative until inode/directory capabilities are introduced.
- PS/2 input is polled, not interrupt-driven. This is enough for the first terminal path; a later IOAPIC/input pass should move it to IRQ delivery.
- Spring parses the manifest only as a seeded status file for now. Real launchd-style manifest parsing is still pending.
