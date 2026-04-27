#!/usr/bin/env bash
# Boot build/disk.img under QEMU with UEFI firmware.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DISK="$ROOT/build/disk.img"

OVMF_CODE="${OVMF_CODE:-}"
OVMF_VARS="${OVMF_VARS:-}"

find_ovmf() {
    for p in \
        /usr/share/OVMF/OVMF_CODE.fd \
        /usr/share/OVMF/OVMF_CODE_4M.fd \
        /usr/share/ovmf/OVMF.fd \
        /usr/share/edk2-ovmf/x64/OVMF_CODE.fd \
        /usr/share/qemu/OVMF.fd; do
        [[ -f "$p" ]] && echo "$p" && return 0
    done
    return 1
}

find_ovmf_vars() {
    for p in \
        /usr/share/OVMF/OVMF_VARS_4M.fd \
        /usr/share/OVMF/OVMF_VARS.fd; do
        [[ -f "$p" ]] && echo "$p" && return 0
    done
    return 1
}
if [[ -z "$OVMF_VARS" ]]; then
    OVMF_VARS="$(find_ovmf_vars || true)"
fi

if [[ -z "$OVMF_CODE" ]]; then
    OVMF_CODE="$(find_ovmf || true)"
fi
if [[ -z "$OVMF_CODE" ]]; then
    echo "OVMF firmware not found; install 'ovmf' or set OVMF_CODE=/path/to/OVMF_CODE.fd" >&2
    exit 1
fi

# If an OVMF_VARS file is provided, treat OVMF_CODE as the read-only code
# half of a split image. Otherwise assume OVMF_CODE is a combined image
# (e.g. /usr/share/ovmf/OVMF.fd) and keep it writable so firmware can stash
# NVRAM without aborting.
CODE_RO="on"
if [[ -z "$OVMF_VARS" || ! -f "$OVMF_VARS" ]]; then
    CODE_RO="off"
fi

POND="$ROOT/build/pond.img"

QEMU_ARGS=(
    -machine q35
    -cpu qemu64
    -m 512M
    -drive if=pflash,format=raw,readonly="$CODE_RO",file="$OVMF_CODE"
    -drive format=raw,file="$DISK"
    -netdev user,id=net0
    -device e1000,netdev=net0,mac=52:54:00:12:34:56
    -audiodev wav,id=audio0,path="$ROOT/build/audio.wav"
    -device AC97,audiodev=audio0
    -serial stdio
    -no-reboot
    -no-shutdown
)

# Attach the PondFS backing image via a QEMU NVMe controller. Gated by the
# file existing so local dev checkouts without a built image still boot.
if [[ -f "$POND" ]]; then
    QEMU_ARGS+=(
        -drive "if=none,id=pond,format=raw,file=$POND"
        -device nvme,drive=pond,serial=personaos
    )
fi

if [[ -n "$OVMF_VARS" && -f "$OVMF_VARS" ]]; then
    # Some distros split code/vars; copy vars to build/ so it's writable.
    mkdir -p "$ROOT/build"
    cp "$OVMF_VARS" "$ROOT/build/OVMF_VARS.fd"
    QEMU_ARGS+=(-drive if=pflash,format=raw,file="$ROOT/build/OVMF_VARS.fd")
fi

if [[ "${1:-}" == "debug" ]]; then
    QEMU_ARGS+=(-s -S)
fi

exec qemu-system-x86_64 "${QEMU_ARGS[@]}"
