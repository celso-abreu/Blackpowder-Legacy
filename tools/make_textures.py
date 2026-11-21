#!/usr/bin/env python3
"""Generate Blackpowder Legacy's 16x16 item textures (deterministic pixel art).

Run:  python3 tools/make_textures.py
Writes PNGs into assets/blackpowder_legacy/textures/item/.
All art is original, drawn here pixel by pixel — no Mojang/Microsoft assets.
"""
from pathlib import Path

from PIL import Image

OUT = Path(__file__).resolve().parent.parent / "assets" / "blackpowder_legacy" / "textures" / "item"

PALETTE = {
    "H": (180, 180, 190, 255),  # sight bead (light steel)
    "l": (40, 40, 45, 255),     # hammer / lever (near-black steel)
    "t": (40, 40, 45, 255),     # trigger (near-black steel)
    "S": (85, 85, 95, 255),     # steel
    "s": (55, 55, 62, 255),     # dark steel
    "w": (124, 84, 50, 255),    # walnut
    "d": (86, 56, 34, 255),     # dark walnut
    "W": (148, 103, 62, 255),   # light walnut
    "C": (184, 115, 51, 255),   # copper
    "J": (160, 90, 45, 255),    # copper jacket (shadowed)
    "B": (201, 162, 39, 255),   # brass
    "b": (232, 199, 102, 255),  # brass highlight
    "R": (143, 107, 45, 255),   # case rim
    "L": (138, 138, 138, 255),  # lead
    "g": (181, 181, 181, 255),  # lead highlight
}

WINCHESTER = [
    "................",
    "................",
    "................",
    "................",
    "................",
    "..............H.",
    ".....l..........",
    ".....lSSSSSSSSS.",
    ".....SSSsssssss.",
    "...wwSSSSwwwww..",
    "..wwwtSll.......",
    ".wwww..l........",
    ".ddd...ll.......",
    ".ddd............",
    "................",
    "................",
]

CARTRIDGE = [
    "................",
    "................",
    ".......CC.......",
    ".......CC.......",
    ".......JC.......",
    ".......JJ.......",
    "......BBJJ......",
    "......BbBB......",
    "......BbBB......",
    "......BbBB......",
    "......BbBB......",
    "......BbBB......",
    "......BBBB......",
    ".....RRRRRR.....",
    "................",
    "................",
]

BULLET = [
    "................",
    "................",
    "................",
    "................",
    "................",
    ".......LL.......",
    "......LgLL......",
    "......LLLL......",
    "......LLLL......",
    "......JJJJ......",
    "................",
    "................",
    "................",
    "................",
    "................",
    "................",
]


def draw(name: str, rows: list[str]) -> None:
    assert len(rows) == 16, f"{name}: {len(rows)} rows, want 16"
    img = Image.new("RGBA", (16, 16), (0, 0, 0, 0))
    for y, row in enumerate(rows):
        assert len(row) == 16, f"{name} row {y}: {len(row)} cols, want 16"
        for x, ch in enumerate(row):
            if ch != ".":
                img.putpixel((x, y), PALETTE[ch])
    path = OUT / f"{name}.png"
    img.save(path)
    print(f"wrote {path}")


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    draw("winchester_1873", WINCHESTER)
    draw("cartridge_44", CARTRIDGE)
    draw("bullet", BULLET)


if __name__ == "__main__":
    main()
