#!/usr/bin/env python3
"""Render the editable SVG DMG artwork to 1× and 2× PNGs.

Install CairoSVG 2.8.2 in a temporary virtual environment before running this
maintenance script. The release jobs use the checked-in PNGs, not CairoSVG.
"""
from pathlib import Path

import cairosvg

ROOT = Path(__file__).resolve().parents[2]
source = ROOT / "assets/dmg/install-background.svg"
for scale in (1, 2):
    suffix = "" if scale == 1 else "@2x"
    destination = ROOT / f"assets/dmg/install-background{suffix}.png"
    cairosvg.svg2png(url=str(source), write_to=str(destination),
                     output_width=680 * scale, output_height=398 * scale)
    print(destination)
