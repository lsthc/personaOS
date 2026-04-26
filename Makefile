# personaOS top-level build orchestration.

CARGO ?= cargo
BUILD_DIR := build

.PHONY: all build build-bootloader build-kernel build-init disk run debug clean fmt clippy check changelog

all: build

# user/init must be built before the kernel because the kernel
# `include_bytes!`es the resulting ELF into its image.
build: build-init build-bootloader build-kernel

build-bootloader:
	cd bootloader && $(CARGO) build

build-kernel:
	cd kernel && $(CARGO) build

build-init:
	cd user/init && $(CARGO) build --release

disk: build
	./tools/mkdisk.sh

run: disk
	./tools/run-qemu.sh

debug: disk
	./tools/run-qemu.sh debug

check:
	cd bootloader && $(CARGO) check
	cd kernel     && $(CARGO) check

clippy:
	cd bootloader && $(CARGO) clippy -- -D warnings
	cd kernel     && $(CARGO) clippy -- -D warnings

fmt:
	$(CARGO) fmt --all

changelog:
	./tools/gen-changelog-manifest.sh

clean:
	$(CARGO) clean
	rm -rf $(BUILD_DIR)
