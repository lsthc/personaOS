# personaOS top-level build orchestration.

CARGO ?= cargo
BUILD_DIR := build

.PHONY: all build build-bootloader build-kernel build-init build-vfsd build-netd build-audiod build-shore build-depth build-reflection build-surface-demo build-desktop build-skim build-stones build-tide build-drift disk run debug clean fmt clippy check changelog

all: build

# user/init must be built before the kernel because the kernel
# `include_bytes!`es the resulting ELF into its image.
build: build-init build-vfsd build-netd build-audiod build-shore build-depth build-reflection build-surface-demo build-desktop build-skim build-stones build-tide build-drift build-bootloader build-kernel

build-bootloader:
	cd bootloader && $(CARGO) build

build-kernel:
	cd kernel && $(CARGO) build

build-init:
	cd user/init && $(CARGO) build --release

build-vfsd:
	cd user/vfsd && $(CARGO) build --release

build-netd:
	cd user/netd && $(CARGO) build --release

build-audiod:
	cd user/audiod && $(CARGO) build --release

build-shore:
	cd user/shore && $(CARGO) build --release

build-depth:
	cd user/depth && $(CARGO) build --release

build-reflection:
	cd user/reflection && $(CARGO) build --release

build-surface-demo:
	cd user/surface-demo && $(CARGO) build --release

build-desktop:
	cd user/desktop && $(CARGO) build --release

build-skim:
	cd user/skim && $(CARGO) build --release

build-stones:
	cd user/stones && $(CARGO) build --release

build-tide:
	cd user/tide && $(CARGO) build --release

build-drift:
	cd user/drift && $(CARGO) build --release

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
