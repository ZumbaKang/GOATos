//! Builds bootable BIOS and UEFI disk images from the compiled `kernel`
//! artifact using the `bootloader` crate. See
//! https://github.com/rust-osdev/bootloader/blob/main/docs/create-disk-image.md

use std::path::PathBuf;

fn main() {
    // Set by cargo; build scripts should write their output here.
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    // Set by cargo's artifact-dependency feature (see `[build-dependencies]`
    // in Cargo.toml and `.cargo/config.toml`'s `bindeps` flag).
    let kernel = PathBuf::from(std::env::var_os("CARGO_BIN_FILE_KERNEL_kernel").unwrap());

    // Create a UEFI disk image for future use (not booted by `src/main.rs` yet).
    let uefi_path = out_dir.join("uefi.img");
    bootloader::UefiBoot::new(&kernel)
        .create_disk_image(&uefi_path)
        .unwrap();

    // Create the BIOS disk image that `src/main.rs` boots by default.
    let bios_path = out_dir.join("bios.img");
    bootloader::BiosBoot::new(&kernel)
        .create_disk_image(&bios_path)
        .unwrap();

    // Expose the image paths to `src/main.rs` via `env!`.
    println!("cargo:rustc-env=UEFI_PATH={}", uefi_path.display());
    println!("cargo:rustc-env=BIOS_PATH={}", bios_path.display());
}
