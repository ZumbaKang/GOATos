//! Minimal on-disk filesystem ("GOATFS") for roadmap 4.5.
//!
//! Lives at a fixed LBA ([`FS_BASE_LBA`]) past the boot sector + kernel so the
//! growing kernel image cannot collide with it. Layout (all little-endian):
//!
//! ```text
//! LBA FS_BASE_LBA + 0  superblock (magic, version, file_count)
//! LBA FS_BASE_LBA + 1  directory (up to MAX_FILES entries of 64 bytes)
//! LBA FS_BASE_LBA + 2… file data sectors
//! ```
//!
//! Enough to mount at boot, look up a file by name, and feed `cat` - not a
//! general-purpose VFS. The disk image is packed by `scripts/build-goatfs.py`.

use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};

use spin::Mutex;

use crate::ata::{self, AtaError, SECTOR_SIZE};

/// Absolute LBA of the GOATFS superblock. Must match the `seek=` used when
/// the Makefile installs `build/goatfs.img` into `build/disk.img`.
pub const FS_BASE_LBA: u32 = 2048;

/// On-disk magic (`GOATFS01`).
const MAGIC: &[u8; 8] = b"GOATFS01";
const VERSION: u32 = 1;

/// Directory capacity (one sector of 64-byte entries).
pub const MAX_FILES: usize = 8;
/// Max bytes a single file may occupy (keeps `cat` buffers bounded).
pub const MAX_FILE_SIZE: usize = 4096;

const NAME_LEN: usize = 32;
const DIR_ENTRY_SIZE: usize = 64;
const SUPERBLOCK_SECTOR: u32 = 0;
const DIRECTORY_SECTOR: u32 = 1;

/// Contents of the test file the image builder plants as `hello.txt`.
/// Boot self-test and CI both check for this exact string.
pub const HELLO_TXT_NAME: &str = "hello.txt";
pub const HELLO_TXT_CONTENTS: &str = "GOATos says hello from disk!\n";

#[derive(Clone, Copy)]
struct DirEntry {
    name: [u8; NAME_LEN],
    name_len: usize,
    size: u32,
    /// Sector offset from [`FS_BASE_LBA`].
    lba_off: u32,
}

struct Mount {
    files: [DirEntry; MAX_FILES],
    file_count: usize,
}

static MOUNTED: AtomicBool = AtomicBool::new(false);
static FS: Mutex<Mount> = Mutex::new(Mount {
    files: [DirEntry {
        name: [0; NAME_LEN],
        name_len: 0,
        size: 0,
        lba_off: 0,
    }; MAX_FILES],
    file_count: 0,
});

/// Why mount / read failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsError {
    Ata(AtaError),
    BadSuperblock,
    BadDirectory,
    NotMounted,
    NotFound,
    TooLarge,
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ata(e) => write!(f, "ata: {e}"),
            Self::BadSuperblock => f.write_str("bad superblock"),
            Self::BadDirectory => f.write_str("bad directory"),
            Self::NotMounted => f.write_str("not mounted"),
            Self::NotFound => f.write_str("not found"),
            Self::TooLarge => f.write_str("file too large"),
        }
    }
}

impl From<AtaError> for FsError {
    fn from(value: AtaError) -> Self {
        Self::Ata(value)
    }
}

/// One-line boot banner (also grepped by CI).
pub struct Banner {
    pub mounted: bool,
    pub file_count: usize,
    pub detail: &'static str,
}

impl fmt::Display for Banner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mounted {
            write!(
                f,
                "FS: GOATFS at LBA {} ({} file{}), {}",
                FS_BASE_LBA,
                self.file_count,
                if self.file_count == 1 { "" } else { "s" },
                self.detail
            )
        } else {
            write!(
                f,
                "FS: GOATFS at LBA {} UNAVAILABLE ({})",
                FS_BASE_LBA, self.detail
            )
        }
    }
}

/// Reads the superblock + directory from disk. Soft-fails into an unmounted
/// state so a missing image never wedges boot.
pub fn init() -> Banner {
    MOUNTED.store(false, Ordering::Relaxed);

    match mount() {
        Ok(count) => {
            MOUNTED.store(true, Ordering::Relaxed);
            Banner {
                mounted: true,
                file_count: count,
                detail: "mounted",
            }
        }
        Err(FsError::Ata(AtaError::NotPresent)) => Banner {
            mounted: false,
            file_count: 0,
            detail: "no ata drive",
        },
        Err(FsError::Ata(AtaError::Timeout)) => Banner {
            mounted: false,
            file_count: 0,
            detail: "ata timeout",
        },
        Err(FsError::BadSuperblock) => Banner {
            mounted: false,
            file_count: 0,
            detail: "bad superblock",
        },
        Err(_) => Banner {
            mounted: false,
            file_count: 0,
            detail: "mount failed",
        },
    }
}

fn mount() -> Result<usize, FsError> {
    let mut sector = [0u8; SECTOR_SIZE];
    ata::read_lba(FS_BASE_LBA + SUPERBLOCK_SECTOR, &mut sector)?;

    if &sector[0..8] != MAGIC.as_slice() {
        return Err(FsError::BadSuperblock);
    }
    let version = u32::from_le_bytes(sector[8..12].try_into().unwrap());
    let file_count = u32::from_le_bytes(sector[12..16].try_into().unwrap()) as usize;
    if version != VERSION || file_count > MAX_FILES {
        return Err(FsError::BadSuperblock);
    }

    ata::read_lba(FS_BASE_LBA + DIRECTORY_SECTOR, &mut sector)?;

    let mut mount = FS.lock();
    mount.file_count = 0;
    for i in 0..file_count {
        let off = i * DIR_ENTRY_SIZE;
        let raw = &sector[off..off + DIR_ENTRY_SIZE];
        let name_bytes = &raw[0..NAME_LEN];
        let name_len = name_bytes.iter().position(|&b| b == 0).unwrap_or(NAME_LEN);
        if name_len == 0 {
            return Err(FsError::BadDirectory);
        }
        if !name_bytes[..name_len]
            .iter()
            .all(|&b| matches!(b, b'!'..=b'~'))
        {
            return Err(FsError::BadDirectory);
        }
        let size = u32::from_le_bytes(raw[32..36].try_into().unwrap());
        let lba_off = u32::from_le_bytes(raw[36..40].try_into().unwrap());
        if size as usize > MAX_FILE_SIZE || lba_off < 2 {
            return Err(FsError::BadDirectory);
        }
        let mut name = [0u8; NAME_LEN];
        name[..name_len].copy_from_slice(&name_bytes[..name_len]);
        mount.files[i] = DirEntry {
            name,
            name_len,
            size,
            lba_off,
        };
        mount.file_count += 1;
    }
    Ok(mount.file_count)
}

/// Reads the named file into `buf`, returning the byte length on success.
///
/// `buf` must be at least as large as the file (and at most [`MAX_FILE_SIZE`]
/// is ever requested from disk).
pub fn read_file(name: &str, buf: &mut [u8]) -> Result<usize, FsError> {
    if !MOUNTED.load(Ordering::Relaxed) {
        return Err(FsError::NotMounted);
    }
    if name.is_empty() || name.len() > NAME_LEN {
        return Err(FsError::NotFound);
    }

    let (size, lba_off) = {
        let mount = FS.lock();
        let entry = mount.files[..mount.file_count]
            .iter()
            .find(|e| e.name_len == name.len() && e.name[..e.name_len] == *name.as_bytes())
            .ok_or(FsError::NotFound)?;
        (entry.size as usize, entry.lba_off)
    };

    if size > buf.len() || size > MAX_FILE_SIZE {
        return Err(FsError::TooLarge);
    }
    if size == 0 {
        return Ok(0);
    }

    let sectors = size.div_ceil(SECTOR_SIZE);
    let mut scratch = [0u8; MAX_FILE_SIZE];
    // Round the transfer up to whole sectors into `scratch`, then copy out.
    let transfer = sectors * SECTOR_SIZE;
    ata::read_lba(FS_BASE_LBA + lba_off, &mut scratch[..transfer])?;
    buf[..size].copy_from_slice(&scratch[..size]);
    Ok(size)
}

/// Boot self-test: `cat` the known `hello.txt` and check its bytes.
pub fn self_test() -> SelfTest {
    let mut buf = [0u8; MAX_FILE_SIZE];
    match read_file(HELLO_TXT_NAME, &mut buf) {
        Ok(n) => {
            let expected = HELLO_TXT_CONTENTS.as_bytes();
            if n == expected.len() && buf[..n] == *expected {
                SelfTest {
                    ok: true,
                    detail: "hello.txt ok",
                }
            } else {
                SelfTest {
                    ok: false,
                    detail: "hello.txt contents mismatch",
                }
            }
        }
        Err(FsError::NotMounted) => SelfTest {
            ok: false,
            detail: "not mounted",
        },
        Err(FsError::NotFound) => SelfTest {
            ok: false,
            detail: "hello.txt missing",
        },
        Err(_) => SelfTest {
            ok: false,
            detail: "read failed",
        },
    }
}

/// Result of [`self_test`].
pub struct SelfTest {
    pub ok: bool,
    pub detail: &'static str,
}

impl fmt::Display for SelfTest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ok {
            write!(f, "FS: self-test ok ({})", self.detail)
        } else {
            write!(f, "FS: self-test FAILED ({})", self.detail)
        }
    }
}
