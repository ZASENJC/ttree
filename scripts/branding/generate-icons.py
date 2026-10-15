#!/usr/bin/env python3
"""Rebuild app icons from the monochrome SVG. Needs librsvg; ICNS needs macOS."""
from pathlib import Path
import shutil
import struct
import subprocess
import sys

root = Path(__file__).resolve().parents[2]
icons = root / 'assets/icons'
work = root / 'target/branding'
work.mkdir(parents=True, exist_ok=True)


def render(size, destination):
    subprocess.run(['rsvg-convert', '-w', str(size), '-h', str(size),
                    '-o', str(destination), str(icons / 'icon.svg')], check=True)


for size, filename in [(1024, 'icon.png'), (128, '128x128.png'), (32, '32x32.png')]:
    render(size, icons / filename)
frames = []
for size in (16, 24, 32, 48, 64, 128, 256):
    path = work / f'{size}.png'
    render(size, path)
    frames.append((size, path.read_bytes()))
offset = 6 + 16 * len(frames)
entries = []
for size, data in frames:
    entries.append(struct.pack('<BBBBHHII', size % 256, size % 256, 0, 0, 1, 32, len(data), offset))
    offset += len(data)
(icons / 'icon.ico').write_bytes(struct.pack('<HHH', 0, 1, len(frames)) + b''.join(entries) + b''.join(data for _, data in frames))
if sys.platform == 'darwin':
    iconset = work / 'TTREE.iconset'
    iconset.mkdir(exist_ok=True)
    for size in (16, 32, 128, 256, 512):
        render(size, iconset / f'icon_{size}x{size}.png')
        render(size*2, iconset / f'icon_{size}x{size}@2x.png')
    subprocess.run(['iconutil', '-c', 'icns', str(iconset), '-o', str(icons / 'icon.icns')], check=True)
else:
    print('ICO/PNG ready; regenerate ICNS on macOS before committing.')
