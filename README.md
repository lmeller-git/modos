# Modos

A modular operatings system kernel with a shared address space.

---

## Usage

The following dependencies are required:

* **Rust Toolchain**: `cargo`
* **QEMU**: `qemu-system-x86_64`
* **ISO Tools**: `xorriso`
* **Utilities**: `git`, `curl`, `make`

---

### Quick Start

Build the kernel, fetch the UEFI firmware and Limine bootloader binaries automatically, create a bootable ISO, and run it in QEMU:

```bash
make run
```
---

### Build & Run

Commands:
* make / make: all Compiles the kernel and builds the bootable tiny_os.iso image.
* make kernel: Compiles only the kernel binary without regenerating the ISO.
* make run: Builds the ISO and launches QEMU with UEFI (OVMF) support.
* make debug: Boots in QEMU and pauses execution, listening for GDB on localhost:1234 (-s -S).
* make check: Runs cargo check targeted at x86_64-unknown-none.
* make clippy: Runs cargo clippy targeted at x86_64-unknown-none.

---

### Testing

The build system supports both host-side unit tests and headless in-kernel integration tests inside QEMU.

```bash
# Run both host unit tests and headless QEMU integration tests
make test

# Run only in-kernel tests in QEMU (headless, builds with `--features test_run`)
make test-qemu

# Run in-kernel tests and wait for GDB connection
make debug-test

# Run host-side package unit tests
make test-host HOST_TEST_PKGS="<pkg>"
```
---

### Configuration

Fine-tune your build by passing variables directly to `make`:

* `QEMUFLAGS`: Append custom flags to the QEMU instance.
* `CARGO_FLAGS`: Pass extra arguments down to Cargo.
* `RUST_PROFILE`: Switch profiles (e.g., `dev` vs `release`).
* `KVM`: Qemu kvm acceleration.

---

### Clean Up

```bash
# Remove build artifacts and generated ISO images
make clean

# Full reset: removes build artifacts, cached Limine binaries, and OVMF firmware
make distclean
```
