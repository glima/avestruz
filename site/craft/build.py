#!/usr/bin/env python3
"""Make the scene's props: `build.py [--bare | --pack] [NAME…]`.

Writes `site/assets/models/NAME.glb` for each prop named, or for all of
them and the ground they stand on (`ground`: `site/assets/tex/sand_*.webp`),
with the working files and a picture of each bare model under
`target/site-craft/`. `--bare` stops after SolidCraft, to look at shapes;
`--pack` only packs again what TextureCraft wrote the last time.
"""

from __future__ import annotations

import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from craft import model, pack, texture, tile
from models import LOOKS, MODELS

SITE = Path(__file__).resolve().parent.parent
WORK = SITE.parent / "target" / "site-craft"
OUT = SITE / "assets" / "models"
TILES = SITE / "assets" / "tex"
# What the plain is made of, and the side of its repeating square in pixels.
GROUND = ("sand", 1024)
# No texture is drawn larger than this, whatever TextureCraft painted it at.
LARGEST = 512


def build(name: str, stage: str) -> str:
    if name == "ground":
        if stage == "all":
            tile(*GROUND, WORK, TILES, "sand")
        return "ground: " + ", ".join(f"{p.name} {p.stat().st_size // 1024} KiB" for p in sorted(TILES.glob("sand_*.webp")))
    bare, textured = WORK / f"{name}.glb", WORK / f"{name}.textured.glb"
    if stage != "pack":
        model(MODELS[name](), LOOKS, bare, WORK / f"{name}.png")
        if stage == "bare":
            return f"{name}: bare"
        texture(LOOKS, bare, textured)
    packed = OUT / f"{name}.glb"
    pack(str(SITE / "node_modules" / ".bin" / "gltf-transform"), textured, packed, LARGEST)
    return f"{name}: {packed.stat().st_size // 1024} KiB"


def main() -> None:
    arguments = sys.argv[1:]
    stage = "bare" if "--bare" in arguments else "pack" if "--pack" in arguments else "all"
    names = [a for a in arguments if not a.startswith("--")] or [*MODELS, "ground"]
    unknown = [n for n in names if n not in MODELS and n != "ground"]
    if unknown:
        sys.exit(f"no such prop: {', '.join(unknown)} (there are: {', '.join(MODELS)})")
    WORK.mkdir(parents=True, exist_ok=True)
    OUT.mkdir(parents=True, exist_ok=True)
    with ThreadPoolExecutor(max_workers=4) as pool:
        for line in pool.map(lambda n: build(n, stage), names):
            print(line)


if __name__ == "__main__":
    main()
