//! Boots the GOATos kernel in QEMU, headlessly.
//!
//! The kernel is compiled and turned into a bootable disk image by
//! `build.rs`; this binary just launches that image with `qemu-system-x86_64`
//! and forwards the emulated COM1 serial port to stdio, since there is no
//! framebuffer output (and no display in most CI/sandbox environments) yet.

use std::env;
use std::process::Command;

fn main() {
    let bios_path = env!("BIOS_PATH");

    let mut cmd = Command::new("qemu-system-x86_64");
    cmd.arg("-drive").arg(format!("format=raw,file={bios_path}"));
    // Forward the kernel's serial output to this process's stdio.
    cmd.arg("-serial").arg("stdio");
    // No GUI window - this is meant to run headlessly (e.g. in a cloud sandbox).
    cmd.arg("-display").arg("none");

    // Forward any extra arguments straight through to QEMU, e.g.:
    //   cargo run -- -no-reboot -no-shutdown
    cmd.args(env::args().skip(1));

    let mut child = cmd.spawn().unwrap_or_else(|err| {
        eprintln!(
            "failed to start `qemu-system-x86_64` ({err}). Is QEMU installed? \
             On Debian/Ubuntu: `sudo apt-get install qemu-system-x86`."
        );
        std::process::exit(1);
    });
    let status = child
        .wait()
        .expect("failed to wait on qemu-system-x86_64");
    std::process::exit(status.code().unwrap_or(1));
}
