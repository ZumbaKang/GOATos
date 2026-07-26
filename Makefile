# Top-level build orchestration for GOATos.
#
# The kernel itself is a plain `cargo build` (see kernel/), but turning it
# into something bootable needs a couple of extra steps `cargo` doesn't know
# about: assembling our own boot sector (boot/boot.asm) and gluing it
# together with the kernel into one raw, bootable disk image. This Makefile
# wires those steps together.
#
# Targets:
#   make            - build the kernel binary only
#   make disk        - build build/disk.img, a bootable raw disk image
#   make run         - build the disk image and boot it in QEMU, headlessly,
#                      with the kernel's serial output forwarded to this
#                      terminal
#   make run-display - like `run`, but with a graphical QEMU window instead
#                      of headless serial-only output
#   make clean       - remove all build artifacts

PROFILE ?= dev
CARGO_PROFILE_FLAG := $(if $(filter release,$(PROFILE)),--release,)
# Cargo's built-in "dev" profile always outputs to a "debug" directory,
# regardless of the profile's name; only custom profiles get their own dir.
PROFILE_DIR := $(if $(filter dev,$(PROFILE)),debug,$(PROFILE))

KERNEL_ELF := kernel/target/i686-goatos/$(PROFILE_DIR)/kernel
BUILD_DIR := build
KERNEL_BIN := $(BUILD_DIR)/kernel.bin
BOOT_BIN := $(BUILD_DIR)/boot.bin
DISK_IMG := $(BUILD_DIR)/disk.img

# Pad the final disk image to a comfortable minimum size: some BIOSes (real
# and emulated) get confused by extremely small "hard disks" when computing
# CHS geometry, even though our own boot code only ever uses LBA addressing.
MIN_DISK_SIZE_BYTES := 10485760 # 10 MiB

.PHONY: all kernel disk run run-display clean

all: kernel

kernel:
	cd kernel && cargo build $(CARGO_PROFILE_FLAG)

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

$(KERNEL_BIN): kernel | $(BUILD_DIR)
	objcopy -O binary $(KERNEL_ELF) $(KERNEL_BIN)

$(DISK_IMG): $(KERNEL_BIN) boot/boot.asm | $(BUILD_DIR)
	$(eval KERNEL_SIZE := $(shell stat -c%s $(KERNEL_BIN)))
	$(eval KERNEL_SECTORS := $(shell echo $$(( ($(KERNEL_SIZE) + 511) / 512 )) ))
	nasm -f bin boot/boot.asm -D KERNEL_SECTORS=$(KERNEL_SECTORS) -o $(BOOT_BIN)
	cat $(BOOT_BIN) $(KERNEL_BIN) > $(DISK_IMG)
	truncate -s $(MIN_DISK_SIZE_BYTES) $(DISK_IMG)

disk: $(DISK_IMG)

run: $(DISK_IMG)
	qemu-system-i386 -drive file=$(DISK_IMG),format=raw -serial stdio -display none

run-display: $(DISK_IMG)
	qemu-system-i386 -drive file=$(DISK_IMG),format=raw -serial stdio

clean:
	rm -rf $(BUILD_DIR)
	rm -rf kernel/target
