---
build: 2
codename: Stillwater
version: 0.0.3
date: 2026-04-26
summary: Warning-free build — kernel and bootloader now pass `cargo clippy -- -D warnings`, plus a stable-feature and raw-pointer cleanup pass.
tags: [bootloader, kernel, tooling]
---

## Highlights

- `make clippy` passes cleanly on both crates under `-D warnings`.
- Removed the leftover `#![feature(naked_functions)]` gate now that the
  feature is stable.
- Cleared the `static_mut_refs` warning on the GDT selector cache by
  routing access through raw pointers — 2024-edition ready.
- Function-pointer → integer conversions now go through `*const ()`,
  silencing `function_casts_as_integer` and `fn_to_numeric_cast`.

## Changes

### bootloader

- **`map_or(true, …)` → `is_none_or(…)`.** Simplified the "pick the
  highest-resolution GOP mode" loop per clippy's `unnecessary_map_or`.
- **Drop hand-rolled `div_ceil`.** `pages_for` now uses
  `u64::div_ceil`.
- **Remove redundant lifetimes on `walk_to_pd` / `walk_to_pt` /
  `ensure_child`.** The borrow checker can infer them, so the explicit
  `'a` was just noise.

### kernel

- **Delete `#![feature(naked_functions)]`.** Stabilised in 1.88.0, the
  gate is no longer required. The `#[unsafe(naked)]` attribute on the
  individual entry points stays.
- **Route `SELECTORS` through raw pointers.** The
  `MaybeUninit<Selectors>` static is now read and written via
  `&raw const/mut` + `core::ptr::read/write`, removing the
  `static_mut_refs` warning. Semantics are unchanged.
- **Clean up function-pointer casts.** `syscall_entry` and `enter_user`
  cast to `*const ()` before `u64` to satisfy
  `function_casts_as_integer`; `spawn_idle`'s `entry` goes through
  `as usize as u64` to avoid `fn_to_numeric_cast`.
- **Replace manual `div_ceil` patterns.** `pmm::init`'s `total_pfns`,
  `bitmap_bytes`, and `bitmap_pages` calculations and
  `user::spawn_init`'s `text_pages` all use `usize::div_ceil` /
  `u64::div_ceil` now.
- **Minor `serial.rs` / `idt.rs` tidy-up.** Dropped the meaningless
  `self.base + 0` in UART init and the `u16 as u16` re-cast in the
  double-fault IST setup.
- **Font module convention.** The lowercase glyph constants
  (`LETTER_a`, `LETTER_b`, …) are deliberately lowercased to match their
  ASCII characters. Added `#![allow(non_upper_case_globals)]` at the
  module level so `nonstandard_style` no longer complains while the
  names stay readable.
- **`#[allow(dead_code)]` on public APIs kept for M1's shape but not yet
  called.** Attribute-only, no logic changes: `inw`/`outw`,
  `SerialPort::write_dec_u32` / `write_fmt_args`, `mm::page_up`,
  `pmm::Pmm::free_one` / `pmm::free_frame`, `vmm::kernel` /
  `AddressSpace::pml4_phys` / `unmap_4k` / `child_mut`,
  `sched::ticks` / `preempt_if_needed`.

## Notes

- `BOOT_INFO_VERSION` stays at 1 — no boot-protocol, ABI, or layout
  changes. Binary compatibility preserved.
- `make build`, `make check`, and `make clippy` all complete with zero
  warnings. Safe to turn `-D warnings` on in CI.
