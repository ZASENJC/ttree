#!/usr/bin/env python3
"""Generate bundle icons and transparent tray glyphs from one SVG master."""
from copy import deepcopy
from pathlib import Path
import struct
import subprocess
import sys
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]
ICONS = ROOT / "assets/icons"
WORK = ROOT / "target/branding"
WORK.mkdir(parents=True, exist_ok=True)


def render(source, size, destination):
    subprocess.run(["rsvg-convert", "-w", str(size), "-h", str(size),
                    "-o", str(destination), str(source)], check=True)


def ico(source, destination):
    frames = []
    for size in (16, 24, 32, 48, 64, 128, 256):
        path = WORK / f"{size}.png"
        render(source, size, path)
        frames.append((size, path.read_bytes()))
    offset = 6 + 16 * len(frames)
    entries = []
    for size, data in frames:
        entries.append(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0,
                                   1, 32, len(data), offset))
        offset += len(data)
    destination.write_bytes(struct.pack("<HHH", 0, 1, len(frames)) + b"".join(entries)
                            + b"".join(data for _, data in frames))


master = ICONS / "icon.svg"
for size, filename in [(1024, "icon.png"), (128, "128x128.png"), (32, "32x32.png")]:
    render(master, size, ICONS / filename)
ico(master, ICONS / "icon.ico")

# Extract the letter paths only; the app icon's dark backing is never copied.
namespace = "http://www.w3.org/2000/svg"
ET.register_namespace("", namespace)
glyph = ET.parse(master).getroot().find(f"{{{namespace}}}g")
for color, name in [("#191919", "light"), ("#ffffff", "dark")]:
    svg = ET.Element(f"{{{namespace}}}svg", {"viewBox": "245 245 534 534", "width": "64", "height": "64"})
    paths = deepcopy(glyph)
    paths.set("fill", color)
    svg.append(paths)
    source = WORK / f"tray-{name}.svg"
    ET.ElementTree(svg).write(source, encoding="unicode")
    ico(source, ICONS / f"tray-{name}.ico")
    if name == "light":
        (ICONS / "mark.svg").write_text(source.read_text())
        render(source, 64, ICONS / "status-template.png")

if sys.platform == "darwin":
    iconset = WORK / "TTREE.iconset"
    iconset.mkdir(exist_ok=True)
    for size in (16, 32, 128, 256, 512):
        render(master, size, iconset / f"icon_{size}x{size}.png")
        render(master, size*2, iconset / f"icon_{size}x{size}@2x.png")
    subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(ICONS / "icon.icns")], check=True)
else:
    print("ICO/PNG ready; regenerate ICNS on macOS before committing.")
