//! ATA/IDE PIO disk driver - primary bus, master drive, 28-bit LBA reads.
//!
//! The bootloader already reads the kernel off disk via BIOS CHS (`INT 13h`),
//! but once protected mode is on there is no BIOS callback path. Anything the
//! kernel wants from the disk after that - starting with the tiny on-disk
//! filesystem in [`crate::fs`] - has to talk to the ATA controller's I/O
//! ports directly.
//!
//! This first cut is polling PIO only (no IRQ14): issue a READ SECTORS
//! command, wait for DRQ with a bounded spin, then pull 256 words per
//! sector from the data port. Failures time out and return an error rather
//! than hanging the kernel (see the defensive-driver pattern in
//! `qemu-testing-and-verification`).

use core::arch::asm;
use core::fmt;

use spin::Mutex;

/// Primary ATA bus I/O ports (legacy ISA compatibility mapping).
const DATA: u16 = 0x1f0;
const SECTOR_COUNT: u16 = 0x1f2;
const LBA_LO: u16 = 0x1f3;
const LBA_MID: u16 = 0x1f4;
const LBA_HI: u16 = 0x1f5;
const DRIVE_HEAD: u16 = 0x1f6;
const STATUS_CMD: u16 = 0x1f7;

/// Status register bits.
const STATUS_ERR: u8 = 1 << 0;
const STATUS_DRQ: u8 = 1 << 3;
const STATUS_DF: u8 = 1 << 5;
const STATUS_BSY: u8 = 1 << 7;

/// ATA READ SECTORS (PIO, with retry).
const CMD_READ_SECTORS: u8 = 0x20;

/// Bytes in one ATA sector.
pub const SECTOR_SIZE: usize = 512;

/// Bound on status-register polls so a missing/wedged controller cannot hang
/// the kernel. Comfortably longer than a real QEMU/v86 PIO transfer.
const SPIN_LIMIT: u32 = 2_000_000;

/// Select the primary master with 28-bit LBA addressing (bit 6 = LBA).
const DRIVE_MASTER_LBA: u8 = 0xe0;

/// Serializes port I/O so two callers cannot interleave command sequences.
static ATA: Mutex<AtaController> = Mutex::new(AtaController { present: false });

struct AtaController {
    present: bool,
}

/// Why a read failed. Reported on the boot banner / `cat` error path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtaError {
    /// Status port read as `0xff` - no device on the bus.
    NotPresent,
    /// Timed out waiting for BSY to clear or DRQ to assert.
    Timeout,
    /// Drive raised ERR or DF during the transfer.
    DriveError,
    /// Caller buffer was not a multiple of [`SECTOR_SIZE`], or LBA out of
    /// 28-bit range / sector count zero.
    BadArgument,
}

impl fmt::Display for AtaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotPresent => "not present",
            Self::Timeout => "timeout",
            Self::DriveError => "drive error",
            Self::BadArgument => "bad argument",
        })
    }
}

/// Probes the primary master. Infallible from the caller's perspective: a
/// missing drive just leaves later reads returning [`AtaError::NotPresent`].
pub fn init() {
    let mut ata = ATA.lock();
    ata.present = false;

    // SAFETY: these ports belong to the primary ATA bus on a PC; selecting
    // the master and reading status is the documented probe sequence and has
    // no side effects beyond what firmware already does at boot.
    unsafe {
        outb(DRIVE_HEAD, DRIVE_MASTER_LBA);
        // Tiny settle delay after a drive select (400ns on real hardware;
        // a handful of port reads is enough under QEMU/v86).
        for _ in 0..4 {
            let _ = inb(STATUS_CMD);
        }
        let status = inb(STATUS_CMD);
        if status == 0xff {
            return;
        }
        // Wait for BSY to drop so we know the controller is accepting commands.
        if wait_while(STATUS_BSY, STATUS_BSY).is_err() {
            return;
        }
    }

    ata.present = true;
}

/// `true` after a successful [`init`] probe.
pub fn is_present() -> bool {
    ATA.lock().present
}

/// Reads `buf.len() / 512` contiguous sectors starting at 28-bit `lba` into
/// `buf`. `buf.len()` must be a non-zero multiple of [`SECTOR_SIZE`].
pub fn read_lba(lba: u32, buf: &mut [u8]) -> Result<(), AtaError> {
    if buf.is_empty() || !buf.len().is_multiple_of(SECTOR_SIZE) {
        return Err(AtaError::BadArgument);
    }
    let sector_count = buf.len() / SECTOR_SIZE;
    if sector_count > 256 || (lba & !0x0fff_ffff) != 0 {
        return Err(AtaError::BadArgument);
    }
    // ATA encodes a 256-sector transfer as a count byte of 0.
    if sector_count == 0 {
        return Err(AtaError::BadArgument);
    }

    let mut ata = ATA.lock();
    if !ata.present {
        return Err(AtaError::NotPresent);
    }

    // SAFETY: primary ATA ports; command sequence is the standard 28-bit LBA
    // PIO read. The lock above keeps another caller from interleaving.
    unsafe { ata.read_pio(lba, sector_count, buf) }
}

impl AtaController {
    unsafe fn read_pio(
        &mut self,
        lba: u32,
        sector_count: usize,
        buf: &mut [u8],
    ) -> Result<(), AtaError> {
        let count_byte = if sector_count == 256 {
            0u8
        } else {
            sector_count as u8
        };

        unsafe {
            if wait_while(STATUS_BSY, STATUS_BSY).is_err() {
                return Err(AtaError::Timeout);
            }

            outb(
                DRIVE_HEAD,
                DRIVE_MASTER_LBA | ((lba >> 24) as u8 & 0x0f),
            );
            outb(SECTOR_COUNT, count_byte);
            outb(LBA_LO, lba as u8);
            outb(LBA_MID, (lba >> 8) as u8);
            outb(LBA_HI, (lba >> 16) as u8);
            outb(STATUS_CMD, CMD_READ_SECTORS);

            for sector in 0..sector_count {
                // Wait until the drive has a sector ready (DRQ) and is not busy.
                if wait_for_drq().is_err() {
                    return Err(AtaError::Timeout);
                }
                let status = inb(STATUS_CMD);
                if status & (STATUS_ERR | STATUS_DF) != 0 {
                    return Err(AtaError::DriveError);
                }

                let start = sector * SECTOR_SIZE;
                let chunk = &mut buf[start..start + SECTOR_SIZE];
                for word in chunk.as_chunks_mut::<2>().0 {
                    let value = inw(DATA);
                    word[0] = value as u8;
                    word[1] = (value >> 8) as u8;
                }
            }
        }

        Ok(())
    }
}

/// Spins until `(status & mask) != value`, or [`SPIN_LIMIT`].
unsafe fn wait_while(mask: u8, value: u8) -> Result<(), AtaError> {
    for _ in 0..SPIN_LIMIT {
        let status = unsafe { inb(STATUS_CMD) };
        if status == 0xff {
            return Err(AtaError::NotPresent);
        }
        if status & mask != value {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err(AtaError::Timeout)
}

/// Waits for BSY clear and DRQ set (a sector is ready to transfer).
unsafe fn wait_for_drq() -> Result<(), AtaError> {
    for _ in 0..SPIN_LIMIT {
        let status = unsafe { inb(STATUS_CMD) };
        if status == 0xff {
            return Err(AtaError::NotPresent);
        }
        if status & STATUS_BSY != 0 {
            core::hint::spin_loop();
            continue;
        }
        if status & (STATUS_ERR | STATUS_DF) != 0 {
            return Err(AtaError::DriveError);
        }
        if status & STATUS_DRQ != 0 {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err(AtaError::Timeout)
}

unsafe fn outb(port: u16, value: u8) {
    unsafe {
        asm!(
            "outb %al, %dx",
            in("dx") port,
            in("al") value,
            options(att_syntax, nomem, nostack, preserves_flags),
        );
    }
}

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    unsafe {
        asm!(
            "inb %dx, %al",
            in("dx") port,
            out("al") value,
            options(att_syntax, nomem, nostack, preserves_flags),
        );
    }
    value
}

unsafe fn inw(port: u16) -> u16 {
    let value: u16;
    unsafe {
        asm!(
            "inw %dx, %ax",
            in("dx") port,
            out("ax") value,
            options(att_syntax, nomem, nostack, preserves_flags),
        );
    }
    value
}
