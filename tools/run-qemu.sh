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
        /usr/share/ovmf/OVMF.fd \
        /usr/share/edk2-ovmf/x64/OVMF_CODE.fd \
        /usr/share/qemu/OVMF.fd; do
        [[ -f "$p" ]] && echo "$p" && return 0
    done
    return 1
}

if [[ -z "$OVMF_CODE" ]]; then
    OVMF_CODE="$(find_ovmf || true)"
fi
if [[ -z "$OVMF_CODE" ]]; then
    echo "OVMF firmware not found; install 'ovmf' or set OVMF_CODE=/path/to/OVMF_CODE.fd" >&2
    exit 1
fi

QEMU_ARGS=(
    -machine q35
    -cpu qemu64
    -m 512M
    -drive if=pflash,format=raw,readonly=on,file="$OVMF_CODE"
    -drive format=raw,file="$DISK"
    -serial stdio
    -no-reboot
    -no-shutdown
)

if [[ -n "$OVMF_VARS" && -f "$OVMF_VARS" ]]; then
    # Some distros split code/vars; copy vars to build/ so it's writable.
    cp "$OVMF_VARS" "$ROOT/build/OVMF_VARS.fd"
    QEMU_ARGS+=(-drive if=pflash,format=raw,file="$ROOT/build/OVMF_VARS.fd")
fi

if [[ "${1:-}" == "debug" ]]; then
    QEMU_ARGS+=(-s -S)
fi

exec qemu-system-x86_64 "${QEMU_ARGS[@]}"
