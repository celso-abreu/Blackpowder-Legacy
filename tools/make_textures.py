#!/usr/bin/env python3
"""Generate Blackpowder Legacy's 16x16 item textures (deterministic pixel art).

Run:  python3 tools/make_textures.py
Writes PNGs into assets/blackpowder_legacy/textures/item/.
All art is original, drawn here pixel by pixel — no Mojang/Microsoft assets.
"""
from pathlib import Path

from PIL import Image

ASSETS = Path(__file__).resolve().parent.parent / "assets" / "blackpowder_legacy" / "textures"
OUT = ASSETS / "item"
ENTITY_OUT = ASSETS / "entity"

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


# The first-person hand, if the player layer cannot be baked: a gloved fist in
# 4x4x12 bounds (a 4x4x4 fist over a 4x4x8 forearm).  Vanilla's box UVs put the
# six faces in bands as tall as the part's cross-section (size.y + size.z = 8
# for this shape), so the sheet is laid out in 8-row bands.

HAND_PALETTE = {
    "k": (196, 140, 103, 255),  # leather, lit
    "l": (168, 113, 79, 255),   # leather, mid
    "d": (128, 84, 58, 255),    # leather, shadow
    "s": (86, 52, 33, 255),     # seam
    "b": (201, 162, 39, 255),   # brass keeper on the cuff
}

# One 4x4 face: lit middle, dark seam round it.
FIST = [
    "slls",
    "lkkd",
    "lkld",
    "slds",
]
# The palm side, flatter and darker.
PALM = [
    "slls",
    "lldd",
    "lldd",
    "slds",
]
# A 4-wide side run for the forearm: a brass keeper two rows in, then leather.
FOREARM = [
    "lkkd",
    "lkld",
    "bbbb",
    "lkld",
    "lkld",
    "lkld",
    "lkld",
    "snds".replace("n", "d"),
]


def hand_pixels() -> list[list[str]]:
    """The 64x64 hand sheet: four 8-row bands of (size.y + size.z) rows."""
    sheet = [["."] * 64 for _ in range(64)]

    def put(u: int, v: int, block: list[str]) -> None:
        for row, text in enumerate(block):
            for col, ch in enumerate(text):
                if ch != ".":
                    assert 0 <= v + row < 64 and 0 <= u + col < 64, "hand: off sheet"
                    sheet[v + row][u + col] = ch

    # Band 1, v 0..8: the two 4x4 caps -- down, then up -- and the east side.
    put(0, 0, PALM)
    put(4, 0, FIST)
    put(8, 0, FOREARM)
    put(12, 0, FOREARM)
    # Band 2, v 8..16: the west, then the north and south sides beside it.
    put(0, 8, FOREARM)
    put(4, 8, FOREARM)
    put(8, 8, FOREARM)
    # Bands 3 and 4 repeat the sides down the forearm.
    for band in (16, 24):
        for u in (0, 4, 8):
            put(u, band, FOREARM)
    return sheet


def draw_hand() -> None:
    sheet = hand_pixels()
    img = Image.new("RGBA", (64, 64), (0, 0, 0, 0))
    for v, row in enumerate(sheet):
        assert len(row) == 64, f"hand row {v}: {len(row)} cols, want 64"
        for u, ch in enumerate(row):
            if ch != ".":
                img.putpixel((u, v), HAND_PALETTE[ch])
    ENTITY_OUT.mkdir(parents=True, exist_ok=True)
    path = ENTITY_OUT / "hand.png"
    img.save(path)
    print(f"wrote {path}")


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    ENTITY_OUT.mkdir(parents=True, exist_ok=True)
    # The rifle's texture is the 1024x1024 sheet extracted from
    # models/winchester_1873.bbmodel by tools/bbmodel_convert.py.
    draw("cartridge_44", CARTRIDGE)
    draw("bullet", BULLET)
    draw_hand()


if __name__ == "__main__":
    main()
