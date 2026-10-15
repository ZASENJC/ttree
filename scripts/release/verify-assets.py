#!/usr/bin/env python3
"""Validate the release asset set and the portable Windows executable."""
from pathlib import Path, PurePosixPath
import sys
import zipfile


def verify(directory, version):
    required = {f"TTREE_{version}_universal.dmg", "TTREE_universal.app.tar.gz",
                "TTREE_universal.app.tar.gz.sig", "latest.json", f"TTREE_{version}_windows_x64.zip"}
    allowed = required | {"macos.sha256", "windows.sha256", "SHA256SUMS", "SHA256SUMS.sig"}
    actual = {p.name for p in directory.iterdir()}
    if not required <= actual or actual - allowed:
        raise ValueError(f"unexpected release files: missing={required-actual}, extra={actual-allowed}")
    for name in required:
        if not (directory / name).is_file() or (directory / name).stat().st_size == 0:
            raise ValueError(f"empty release asset: {name}")
    with zipfile.ZipFile(directory / f"TTREE_{version}_windows_x64.zip") as archive:
        if archive.testzip():
            raise ValueError("damaged Windows ZIP")
        for entry in archive.namelist():
            if ".." in PurePosixPath(entry).parts or entry.startswith(("/", "\\")) or ":" in entry:
                raise ValueError("unsafe Windows ZIP path")
        with archive.open("TTREE/TTREE.exe") as executable:
            if executable.read(2) != b"MZ":
                raise ValueError("Windows executable is missing its PE header")


if __name__ == "__main__":
    verify(Path(sys.argv[1]), sys.argv[2])
