#!/usr/bin/env python3
"""Development-only export of the existing macOS icon (requires Pillow).
Exports stay outside runtime paths. The 256x256 PNG is the macOS Dock icon (a 128 pt tile on Retina) the GUI sets after its first frame. The Sixel asset is a 256-entry RGBA palette
followed by 96x96 indices; Kitty uses the PNG and other terminals the cell export.
"""
from pathlib import Path
from PIL import Image

root = Path(__file__).resolve().parent.parent
source = Image.open(root / 'assets/lenz-icon.png').convert('RGBA')
source.resize((96, 96), Image.Resampling.LANCZOS).save(root / 'assets/lenz-about.png', optimize=True)
source.resize((256, 256), Image.Resampling.LANCZOS).save(root / 'assets/lenz-dock.png', optimize=True)
(root / 'assets/lenz-about-terminal.rgba').write_bytes(
    source.resize((24, 24), Image.Resampling.LANCZOS).tobytes())
indexed = source.resize((96, 96), Image.Resampling.LANCZOS).quantize(
    colors=256, method=Image.Quantize.FASTOCTREE)
palette = bytes(indexed.getpalette('RGBA'))
asset = palette + bytes(1024 - len(palette)) + indexed.tobytes()
assert len(asset) == 1024 + 96 * 96
(root / 'assets/lenz-about-sixel.bin').write_bytes(asset)
