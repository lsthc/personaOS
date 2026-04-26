# personaOS top-level build orchestration.

CARGO ?= cargo
BUILD_DIR := build

.PHONY: all build build-bootloader build-kernel disk run debug clean fmt clippy check

all: build

build: build-bootloader build-kernel

build-bootloader:
	cd bootloader && $(CARGO) build

build-kernel:
	cd kernel && $(CARGO) build

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

clean:
	$(CARGO) clean
	rm -rf $(BUILD_DIR)
