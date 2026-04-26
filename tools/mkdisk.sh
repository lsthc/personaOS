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
POND_IMG="$BUILD/pond.img"
ESP_SIZE_MB=64
DISK_SIZE_MB=96
POND_SIZE_MB=32

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

# Partition 1 starts at LBA 2048 because that's what we passed to --new
# above. Avoid parsing `sgdisk --info` whose output varies by version.
START_LBA=2048
dd if="$ESP_IMG" of="$DISK_IMG" bs=512 seek="$START_LBA" conv=notrunc status=none

echo "disk image: $DISK_IMG"

# --- Build PondFS backing image + format + seed ----------------------------
MKPONDFS="$ROOT/tools/mkpondfs/target/release/mkpondfs"
if [[ ! -x "$MKPONDFS" ]]; then
    (cd "$ROOT/tools/mkpondfs" && cargo build --release >/dev/null)
fi

POND_SRC="$BUILD/pondfs-src"
rm -rf "$POND_SRC"
mkdir -p "$POND_SRC"

# Stage the payload we want on the filesystem.
cp "$ROOT/user/init/target/x86_64-personaos-user/release/init" "$POND_SRC/init"
printf 'hello from pond\n' > "$POND_SRC/hello.txt"

rm -f "$POND_IMG"
truncate -s ${POND_SIZE_MB}M "$POND_IMG"
"$MKPONDFS" "$POND_IMG" "$POND_SRC"
echo "pond image: $POND_IMG"
