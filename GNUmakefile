# Build system for the workspace (x86_64, UEFI via Limine).
#
# Host requirements: cargo (toolchain pinned in rust-toolchain.toml),
# qemu-system-x86_64, xorriso, git, curl.

MAKEFLAGS += -rR
.SUFFIXES:

# ---------------------------------------------------------------------------
# user settings
# ---------------------------------------------------------------------------

RUST_PROFILE     ?= dev
CARGO_FLAGS      ?=
CARGO_TARGET_DIR ?= target
IMAGE_NAME       ?= tiny_os
QEMUFLAGS        ?= -m 12G
QEMU_WRAPPER     ?= ./scripts/run_qemu.sh
LIMINE_BRANCH    ?= v9.x-binary

# Set KVM=1 for anything performance related
KVM ?=

HOST_TEST_PKGS ?=

# ---------------------------------------------------------------------------
# Kernel build
# ---------------------------------------------------------------------------

# Cargo package of the kernel binary.
KERNEL_PKG    ?= entry
# Produced binary, same as limine path
KERNEL_BIN    ?= entry
KERNEL_TARGET ?= x86_64-unknown-none

KERNEL_RUSTFLAGS ?= -C relocation-model=static

PROFILE_DIR := $(if $(filter dev,$(RUST_PROFILE)),debug,$(RUST_PROFILE))
KERNEL_ELF  := $(CARGO_TARGET_DIR)/$(KERNEL_TARGET)/$(PROFILE_DIR)/$(KERNEL_BIN)
ISO         := $(IMAGE_NAME).iso

KERNEL_ENV  := RUSTFLAGS="$(KERNEL_RUSTFLAGS)"
KERNEL_ARGS := --target $(KERNEL_TARGET) --profile $(RUST_PROFILE) --target-dir $(CARGO_TARGET_DIR) $(CARGO_FLAGS)

# ---------------------------------------------------------------------------
# Boot environment
# ---------------------------------------------------------------------------

LIMINE_DIR := limine

OVMF_DIR  := ovmf
OVMF_CODE := $(OVMF_DIR)/ovmf-code-x86_64.fd
OVMF_VARS := $(OVMF_DIR)/ovmf-vars-x86_64.fd
OVMF_BASE := https://github.com/osdev0/edk2-ovmf-nightly/releases
# Known-good nightly, used if "latest" cannot be downloaded. Bump now and then.
# TODO: migrate to https://github.com/rust-osdev/ovmf-prebuilt/
OVMF_FALLBACK := nightly-20251203T012444Z

ifneq ($(KVM),)
override QEMUFLAGS += -enable-kvm -cpu host,+invtsc
endif

# ---------------------------------------------------------------------------
# Targets
# ---------------------------------------------------------------------------

.PHONY: all
all: $(ISO)

# Always invoke cargo (it does its own change detection). Make only rebuilds the
# ISO if cargo actually changed the binary.
.PHONY: kernel
kernel: $(KERNEL_ELF)

$(KERNEL_ELF): FORCE
	$(KERNEL_ENV) cargo build -p $(KERNEL_PKG) $(KERNEL_ARGS)

FORCE:

# Type-check / lint everything in default-members for the kernel target.
.PHONY: check
check:
	$(KERNEL_ENV) cargo check $(KERNEL_ARGS)

.PHONY: clippy
clippy:
	$(KERNEL_ENV) cargo clippy $(KERNEL_ARGS)

# UEFI-only bootable ISO.
$(ISO): $(KERNEL_ELF) limine.conf $(LIMINE_DIR)/BOOTX64.EFI
	rm -rf iso_root
	mkdir -p iso_root/boot/limine iso_root/EFI/BOOT
	cp $(KERNEL_ELF) iso_root/boot/$(KERNEL_BIN)
	cp limine.conf iso_root/boot/limine/
	cp $(LIMINE_DIR)/limine-uefi-cd.bin iso_root/boot/limine/
	cp $(LIMINE_DIR)/BOOTX64.EFI iso_root/EFI/BOOT/
	xorriso -as mkisofs \
		--efi-boot boot/limine/limine-uefi-cd.bin \
		-efi-boot-part --efi-boot-image --protective-msdos-label \
		iso_root -o $@
	rm -rf iso_root

$(LIMINE_DIR)/BOOTX64.EFI:
	rm -rf $(LIMINE_DIR)
	git clone https://github.com/limine-bootloader/limine.git --branch=$(LIMINE_BRANCH) --depth=1 $(LIMINE_DIR)

$(OVMF_DIR)/ovmf-%-x86_64.fd:
	mkdir -p $(OVMF_DIR)
	curl -fLo $@ $(OVMF_BASE)/latest/download/$(@F) \
		|| curl -fLo $@ $(OVMF_BASE)/download/$(OVMF_FALLBACK)/$(@F) \
		|| { rm -f $@; echo "OVMF download failed"; exit 1; }

# ---------------------------------------------------------------------------
# Running and testing
# ---------------------------------------------------------------------------

.PHONY: run
run: $(ISO) $(OVMF_CODE) $(OVMF_VARS)
	$(QEMU_WRAPPER) qemu-system-x86_64 \
		-M q35 \
		-drive if=pflash,unit=0,format=raw,file=$(OVMF_CODE),readonly=on \
		-drive if=pflash,unit=1,format=raw,file=$(OVMF_VARS) \
		-cdrom $(ISO) \
		-device isa-debug-exit,iobase=0xf4,iosize=0x04 \
		-serial stdio \
		$(QEMUFLAGS)

.PHONY: debug
debug:
	$(MAKE) run QEMUFLAGS="$(QEMUFLAGS) -s -S"

# In-kernel tests: build with the test_run feature into a separate target dir and
# image so normal and test builds do not clobber each other.
.PHONY: test-qemu
test-qemu:
	$(MAKE) run \
		IMAGE_NAME=$(IMAGE_NAME)-test \
		CARGO_TARGET_DIR=$(CARGO_TARGET_DIR)/test \
		CARGO_FLAGS="$(CARGO_FLAGS) --features test_run" \
		QEMUFLAGS="$(QEMUFLAGS) -display none"

# Test with debug flags for gdb
.PHONY: debug-test
debug-test:
	$(MAKE) test-qemu QEMUFLAGS="$(QEMUFLAGS) -s -S"

# Host-side unit tests (host target)
.PHONY: test-host
test-host:
ifeq ($(strip $(HOST_TEST_PKGS)),)
	@echo "HOST_TEST_PKGS is empty, nothing to test on the host"
else
	cargo test $(addprefix -p ,$(HOST_TEST_PKGS)) --target-dir $(CARGO_TARGET_DIR) $(CARGO_FLAGS)
endif

.PHONY: test
test: test-host test-qemu

# ---------------------------------------------------------------------------
# Housekeeping
# ---------------------------------------------------------------------------

.PHONY: clean
clean:
	cargo clean --target-dir $(CARGO_TARGET_DIR)
	rm -rf iso_root *.iso

.PHONY: distclean
distclean: clean
	rm -rf $(LIMINE_DIR) $(OVMF_DIR)

# ---------------------------------------------------------------------------
# Later
#
#   WASM guest apps:   cargo build -p <app> --target wasm32-wasip1 --release
#   Host tools:        cargo build -p trace-conv           (no --target)
#   Dynamic modules:   same workspace, different RUSTFLAGS (PIC etc.) and its
#                      own --target-dir so the cache does not thrash
# ---------------------------------------------------------------------------
