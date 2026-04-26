#!/usr/bin/env bash
# Build a GPT-partitioned disk image containing an EFI System Partition with
# `personaboot.efi` and `kernel.elf`. Produces build/disk.img.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUILD="$ROOT/build"
mkdir -p "$BUILD"

BOOT_EFI="$ROOT/target/x86_64-unknown-uefi/debug/personaboot.efi"
KERNEL_ELF="$ROOT/target/x86_64-personaos-kernel/debug/kernel"

if [[ ! -f "$BOOT_EFI" ]]; then
    echo "personaboot.efi not found — run 'make build' first" >&2
    exit 1
fi
if [[ ! -f "$KERNEL_ELF" ]]; then
    echo "kernel not found — run 'make build' first" >&2
    exit 1
fi

ESP_IMG="$BUILD/esp.img"
DISK_IMG="$BUILD/disk.img"
ESP_SIZE_MB=64
DISK_SIZE_MB=96

# --- Build ESP (FAT32) -----------------------------------------------------
rm -f "$ESP_IMG"
truncate -s ${ESP_SIZE_MB}M "$ESP_IMG"
mformat -i "$ESP_IMG" -F ::
mmd -i "$ESP_IMG" ::/EFI ::/EFI/BOOT ::/EFI/personaOS

# UEFI default path (so firmware boots without an NVRAM entry).
mcopy -i "$ESP_IMG" "$BOOT_EFI" ::/EFI/BOOT/BOOTX64.EFI
mcopy -i "$ESP_IMG" "$BOOT_EFI" ::/EFI/personaOS/personaboot.efi
mcopy -i "$ESP_IMG" "$KERNEL_ELF" ::/EFI/personaOS/kernel.elf

# --- Build GPT disk --------------------------------------------------------
rm -f "$DISK_IMG"
truncate -s ${DISK_SIZE_MB}M "$DISK_IMG"
sgdisk --clear \
       --new=1:2048:+${ESP_SIZE_MB}M \
       --typecode=1:EF00 \
       --change-name=1:"EFI System" \
       "$DISK_IMG" >/dev/null

# Copy the ESP into partition 1. `sgdisk --info=1` prints start LBA.
START_LBA=$(sgdisk --info=1 "$DISK_IMG" | awk '/First sector/ {print $3}')
dd if="$ESP_IMG" of="$DISK_IMG" bs=512 seek="$START_LBA" conv=notrunc status=none

echo "disk image: $DISK_IMG"
