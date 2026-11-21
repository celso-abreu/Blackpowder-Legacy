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
    "C": (184, 115, 51, 255),   # copper
    "J": (160, 90, 45, 255),    # copper jacket (shadowed)
    "B": (201, 162, 39, 255),   # brass
    "b": (232, 199, 102, 255),  # brass highlight
    "R": (143, 107, 45, 255),   # case rim
    "L": (138, 138, 138, 255),  # lead
    "g": (181, 181, 181, 255),  # lead highlight
}

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
    # The rifle's texture is the 1024x1024 sheet extracted from
    # models/winchester_1873.bbmodel by tools/bbmodel_convert.py.
    draw("cartridge_44", CARTRIDGE)
    draw("bullet", BULLET)


if __name__ == "__main__":
    main()
