#!/usr/bin/env bash
# One-time host bootstrap for building and running personaOS.
# Installs: rustup (pinned nightly + rust-src), QEMU, OVMF, mtools, sgdisk.
set -euo pipefail

say() { printf '\n\033[1;34m[bootstrap]\033[0m %s\n' "$*"; }

OS="$(uname -s)"

install_apt() {
    say "Installing system packages (apt): qemu, OVMF, mtools, gdisk, parted, build-essential"
    sudo apt-get update
    sudo apt-get install -y --no-install-recommends \
        qemu-system-x86 ovmf mtools gdisk parted \
        build-essential curl ca-certificates xorriso
}

install_rustup() {
    if command -v rustup >/dev/null 2>&1; then
        say "rustup already present — updating"
        rustup self update || true
    else
        say "Installing rustup"
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
            | sh -s -- -y --default-toolchain none --profile minimal
        # shellcheck disable=SC1091
        . "$HOME/.cargo/env"
    fi

    say "Installing the pinned toolchain from rust-toolchain.toml"
    # Running any cargo command inside the project triggers the install.
    (cd "$(dirname "$0")/.." && rustup show active-toolchain)

    say "Adding required target: x86_64-unknown-uefi"
    rustup target add x86_64-unknown-uefi
}

check_ovmf() {
    for p in \
        /usr/share/OVMF/OVMF_CODE.fd \
        /usr/share/ovmf/OVMF.fd \
        /usr/share/edk2-ovmf/x64/OVMF_CODE.fd; do
        if [[ -f "$p" ]]; then
            say "OVMF found at $p"
            return 0
        fi
    done
    say "WARN: OVMF not found — tools/run-qemu.sh will fail until it is."
}

case "$OS" in
    Linux)
        if command -v apt-get >/dev/null 2>&1; then
            install_apt
        else
            say "WARN: non-apt Linux detected. Please install: qemu-system-x86_64, ovmf/edk2-ovmf, mtools, gdisk, parted."
        fi
        ;;
    Darwin)
        say "macOS detected. Install via Homebrew:"
        echo "    brew install qemu mtools gdisk"
        echo "    # OVMF: download from https://github.com/clearlinux/common/raw/master/OVMF.fd"
        ;;
    *)
        say "Unsupported OS: $OS"
        exit 1
        ;;
esac

install_rustup
check_ovmf

say "done. Next:  make build && make run"
