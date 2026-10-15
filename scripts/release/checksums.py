#!/usr/bin/env python3
"""Portable SHA-256 manifests shared by Windows, macOS and publication."""
import argparse
import hashlib
from pathlib import Path


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def write(directory, name):
    paths = sorted(p for p in directory.iterdir()
                   if p.is_file() and p.name not in (name, "SHA256SUMS.sig") and not p.name.endswith(".sha256"))
    if not paths:
        raise ValueError("no assets to checksum")
    (directory / name).write_text("".join(f"{digest(p)}  {p.name}\n" for p in paths))


def check(directory, name):
    lines = (directory / name).read_text().splitlines()
    if not lines:
        raise ValueError("empty checksum manifest")
    for line in lines:
        expected, filename = line.split("  ", 1)
        if Path(filename).name != filename or filename in (".", ".."):
            raise ValueError("unsafe checksum path")
        if digest(directory / filename) != expected:
            raise ValueError(f"checksum mismatch: {filename}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("directory", type=Path)
    parser.add_argument("--write")
    parser.add_argument("--check", action="append", default=[])
    args = parser.parse_args()
    for name in args.check:
        check(args.directory, name)
    if args.write:
        write(args.directory, args.write)
