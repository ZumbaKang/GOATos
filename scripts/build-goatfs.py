#!/usr/bin/env python3
"""Pack a tiny GOATFS image for GOATos (roadmap 4.5).

Layout (must match kernel/src/fs.rs):
  sector 0  superblock: magic GOATFS01, version 1, file_count
  sector 1  directory: up to 8 x 64-byte entries (name[32], size u32, lba_off u32)
  sector 2+ file data

Writes the image to the path given as argv[1] (default: build/goatfs.img).
"""

from __future__ import annotations

import struct
import sys
from pathlib import Path

MAGIC = b"GOATFS01"
VERSION = 1
SECTOR = 512
NAME_LEN = 32
DIR_ENTRY = 64
MAX_FILES = 8

# Keep in sync with kernel/src/fs.rs::HELLO_TXT_CONTENTS.
FILES = [
    ("hello.txt", b"GOATos says hello from disk!\n"),
]


def pack(path: Path) -> None:
    if len(FILES) > MAX_FILES:
        raise SystemExit(f"too many files (max {MAX_FILES})")

    data_lba = 2  # first data sector relative to FS base
    dir_bytes = bytearray(SECTOR)
    data_blobs: list[bytes] = []

    for i, (name, contents) in enumerate(FILES):
        name_b = name.encode("ascii")
        if len(name_b) == 0 or len(name_b) >= NAME_LEN:
            raise SystemExit(f"bad file name: {name!r}")
        if len(contents) > 4096:
            raise SystemExit(f"file too large: {name}")

        entry = bytearray(DIR_ENTRY)
        entry[0 : len(name_b)] = name_b
        struct.pack_into("<II", entry, 32, len(contents), data_lba)
        dir_bytes[i * DIR_ENTRY : (i + 1) * DIR_ENTRY] = entry

        sectors = (len(contents) + SECTOR - 1) // SECTOR
        blob = contents + b"\x00" * (sectors * SECTOR - len(contents))
        data_blobs.append(blob)
        data_lba += sectors

    superblock = bytearray(SECTOR)
    superblock[0:8] = MAGIC
    struct.pack_into("<II", superblock, 8, VERSION, len(FILES))

    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("wb") as f:
        f.write(superblock)
        f.write(dir_bytes)
        for blob in data_blobs:
            f.write(blob)

    print(f"wrote {path} ({path.stat().st_size} bytes, {len(FILES)} file(s))")


def main() -> None:
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("build/goatfs.img")
    pack(out)


if __name__ == "__main__":
    main()
