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
mkdir -p "$POND_SRC/bin" "$POND_SRC/etc" "$POND_SRC/sbin"
cp "$ROOT/user/init/target/x86_64-personaos-user/release/init" "$POND_SRC/init"
cp "$ROOT/user/vfsd/target/x86_64-personaos-user/release/vfsd" "$POND_SRC/sbin/vfsd"
cp "$ROOT/user/netd/target/x86_64-personaos-user/release/netd" "$POND_SRC/sbin/netd"
cp "$ROOT/user/audiod/target/x86_64-personaos-user/release/audiod" "$POND_SRC/sbin/audiod"
cp "$ROOT/user/depth/target/x86_64-personaos-user/release/depth" "$POND_SRC/bin/depth"
cp "$ROOT/user/desktop/target/x86_64-personaos-user/release/desktop" "$POND_SRC/bin/desktop"
cp "$ROOT/user/drift/target/x86_64-personaos-user/release/drift" "$POND_SRC/bin/drift"
cp "$ROOT/user/reflection/target/x86_64-personaos-user/release/reflection" "$POND_SRC/bin/reflection"
cp "$ROOT/user/skim/target/x86_64-personaos-user/release/skim" "$POND_SRC/bin/skim"
cp "$ROOT/user/shore/target/x86_64-personaos-user/release/shore" "$POND_SRC/bin/shore"
cp "$ROOT/user/stones/target/x86_64-personaos-user/release/stones" "$POND_SRC/bin/stones"
cp "$ROOT/user/surface-demo/target/x86_64-personaos-user/release/surface-demo" "$POND_SRC/bin/surface-demo"
cp "$ROOT/user/tide/target/x86_64-personaos-user/release/tide" "$POND_SRC/bin/tide"
printf 'hello from pond\n' > "$POND_SRC/hello.txt"
cat > "$POND_SRC/etc/spring.toml" <<'EOF'
[[service]]
label = "com.persona.vfsd"
path = "/sbin/vfsd"
keep_alive = true

[[service]]
label = "com.persona.netd"
path = "/sbin/netd"
keep_alive = false

[[service]]
label = "com.persona.audiod"
path = "/sbin/audiod"
keep_alive = false

[[service]]
label = "com.persona.reflection"
path = "/bin/reflection"
keep_alive = false

[[service]]
label = "com.persona.desktop"
path = "/bin/desktop"
keep_alive = false

[[service]]
label = "com.persona.tide"
path = "/bin/tide"
keep_alive = false

[[service]]
label = "com.persona.skim"
path = "/bin/skim"
keep_alive = false

[[service]]
label = "com.persona.stones"
path = "/bin/stones"
keep_alive = false

[[service]]
label = "com.persona.drift"
path = "/bin/drift"
keep_alive = false

[[service]]
label = "com.persona.surface-demo"
path = "/bin/surface-demo"
keep_alive = false

[[service]]
label = "com.persona.depth"
path = "/bin/depth"
keep_alive = false
EOF

rm -f "$POND_IMG"
truncate -s ${POND_SIZE_MB}M "$POND_IMG"
"$MKPONDFS" "$POND_IMG" "$POND_SRC"
echo "pond image: $POND_IMG"
