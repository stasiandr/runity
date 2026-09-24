#!/usr/bin/env python3
"""The project's data as one gzipped file for the page to fetch: what the
game reads at run time — scrap.ron, the scenes, prefabs, screens, strings,
the built library — and not what it is built from (sources, raw assets).

    web/pack.py OUT.bin.gz

The layout is `src/web.rs`'s `unpack`: a count, then each file's path and
bytes, every length a little-endian u32.
"""

import gzip
import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
# What the game never reads while it runs.
SKIP_DIRS = {"src", "target", "web", "site", ".scrap"}
SKIP_FILES = {"Cargo.toml", "Cargo.lock", "build.rs", "README.md", "CLAUDE.md"}


def wanted(path: Path) -> bool:
    rel = path.relative_to(ROOT)
    if rel.parts[0] in SKIP_DIRS or rel.name in SKIP_FILES or rel.name.startswith("."):
        return False
    # Raw assets are built into library/; their sidecars (ids) are kept.
    if rel.parts[0] == "assets":
        return rel.suffix == ".scrimport"
    return True


def main() -> None:
    out = Path(sys.argv[1])
    files = sorted(p for p in ROOT.rglob("*") if p.is_file() and wanted(p))
    body = bytearray(struct.pack("<I", len(files)))
    for path in files:
        name = path.relative_to(ROOT).as_posix().encode()
        data = path.read_bytes()
        body += struct.pack("<I", len(name)) + name + struct.pack("<I", len(data)) + data
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_bytes(gzip.compress(bytes(body), 9))
    print(f"{len(files)} files, {len(body) / 1e6:.1f} MB -> {out.stat().st_size / 1e6:.1f} MB")


if __name__ == "__main__":
    main()
