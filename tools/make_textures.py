#!/usr/bin/env python3
"""Generate Blackpowder Legacy's 32x32 item textures (deterministic pixel art).

Run:  python3 tools/make_textures.py
Writes PNGs into assets/blackpowder_legacy/textures/item/ using Python's
standard library only (no Pillow dependency required).
All art is original, drawn here pixel by pixel — no Mojang/Microsoft assets.
"""

from pathlib import Path
import struct
import zlib

OUT = Path(__file__).resolve().parent.parent / "assets" / "blackpowder_legacy" / "textures" / "item"

PALETTE: dict[str, tuple[int, int, int, int]] = {
    "0": (51, 55, 61, 255),     # lead dark outline
    "1": (63, 68, 74, 255),     # lead left rim
    "2": (74, 35, 13, 255),     # copper dark outline
    "3": (74, 53, 12, 255),     # brass dark outline
    "4": (92, 46, 20, 255),     # copper left rim
    "5": (93, 100, 107, 255),   # lead deep shadow
    "6": (94, 69, 18, 255),     # brass left rim
    "7": (120, 88, 20, 255),    # brass deep shadow
    "8": (126, 134, 143, 255),  # lead mid-shadow
    "9": (135, 69, 27, 255),    # copper shadow
    "A": (146, 154, 163, 255),  # lead midtone
    "B": (153, 115, 28, 255),   # brass shadow
    "C": (156, 165, 174, 255),  # lead body
    "D": (181, 99, 43, 255),    # copper mid-shadow
    "E": (189, 146, 42, 255),   # brass warm midtone
    "F": (191, 111, 54, 255),   # copper body
    "G": (191, 149, 40, 255),   # brass body
    "H": (199, 208, 217, 255),  # lead bright
    "I": (217, 176, 56, 255),   # brass bright
    "J": (219, 137, 72, 255),   # copper bright
    "K": (232, 237, 242, 255),  # lead specular highlight
    "L": (242, 173, 114, 255),  # copper specular highlight
    "M": (242, 213, 111, 255),  # brass highlight
    "N": (255, 240, 166, 255),  # brass specular gleam
    # Bullet-hole cutout palette (strictly alpha 255 or '.' alpha 0 so Horizon
    # renders the standalone decal on the fast cutout GPU-instanced path):
    "p": (11, 12, 15, 255),     # deep bullet puncture core
    "q": (22, 21, 20, 255),     # inner charred cavity
    "r": (38, 35, 33, 255),     # gunpowder soot ring
    "s": (60, 56, 52, 255),     # scorched crater bevel
    "t": (86, 82, 77, 255),     # chipped rim highlight
    "u": (98, 104, 112, 255),   # deformed lead glint inside core
}

CARTRIDGE = [
    "................................",
    "................................",
    "................................",
    "..............4LJ2..............",
    ".............4JLJD2.............",
    ".............4FLJD2.............",
    ".............4FLJD2.............",
    ".............4FLJD2.............",
    ".............4FLJD2.............",
    ".............4FLJD2.............",
    ".............29JD92.............",
    ".............6MNIG3.............",
    ".............6ENMG3.............",
    "............6MNMIGB3............",
    "............6ENMIGB3............",
    "............6ENMIGB3............",
    "............6ENMIGB3............",
    "............6ENMIGB3............",
    "............6ENMIGB3............",
    "............6ENMIGB3............",
    "............6ENMIGB3............",
    "............6ENMIGB3............",
    "............6ENMIGB3............",
    "............6ENMIGB3............",
    "............6ENMIGB3............",
    "............6ENMIGB3............",
    "............3BIGB733............",
    "..........6EMNMIGBB773..........",
    "..........37GIGB773333..........",
    "................................",
    "................................",
    "................................",
]

BULLET = [
    "................................",
    "................................",
    "................................",
    "................................",
    "................................",
    "................................",
    "................................",
    "................................",
    "................................",
    "..............1KH0..............",
    ".............1HKHC0.............",
    ".............1AKH80.............",
    "............1AKHC850............",
    "............1AKHC850............",
    "............1AKHC850............",
    "............1AKHC850............",
    "............05C85500............",
    "............1AKHC850............",
    "............1AHC8550............",
    "............4FLJD992............",
    "............29DD9222............",
    "................................",
    "................................",
    "................................",
    "................................",
    "................................",
    "................................",
    "................................",
    "................................",
    "................................",
    "................................",
    "................................",
]

BULLET_HOLE = [
    "................",
    ".......s........",
    ".....sttts......",
    "....srrrrts.....",
    "...srqqqqrrs....",
    "..stqppppqrs....",
    "..srqpupppqts...",
    ".strqpppppqrs...",
    "..srqpppppqrs...",
    "..stqqpppqqs....",
    "...srrqqqrrts...",
    "....strrrrs.....",
    ".....sstss......",
    "........s.......",
    "................",
    "................",
]


def _png_chunk(chunk_type: bytes, data: bytes) -> bytes:
    crc = zlib.crc32(chunk_type + data) & 0xFFFFFFFF
    return struct.pack(">I", len(data)) + chunk_type + data + struct.pack(">I", crc)


def write_png_rgba(path: Path, width: int, height: int, rgba_rows: list[bytes]) -> None:
    raw = b"".join(b"\x00" + row for row in rgba_rows)
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    idat = zlib.compress(raw, level=9)
    data = (
        b"\x89PNG\r\n\x1a\n"
        + _png_chunk(b"IHDR", ihdr)
        + _png_chunk(b"IDAT", idat)
        + _png_chunk(b"IEND", b"")
    )
    path.write_bytes(data)


def draw(name: str, rows: list[str], size: int = 32) -> None:
    assert len(rows) == size, f"{name}: {len(rows)} rows, want {size}"
    rgba_rows: list[bytes] = []
    for y, row in enumerate(rows):
        assert len(row) == size, f"{name} row {y}: {len(row)} cols, want {size}"
        buf = bytearray(size * 4)
        for x, ch in enumerate(row):
            if ch != ".":
                r, g, b, a = PALETTE[ch]
                buf[x * 4 : (x + 1) * 4] = bytes((r, g, b, a))
        rgba_rows.append(bytes(buf))
    path = OUT / f"{name}.png"
    write_png_rgba(path, size, size, rgba_rows)
    print(f"wrote {path} ({size}x{size})")


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    # The rifle's texture is the 1024x1024 sheet extracted from
    # models/winchester_1873.bbmodel by tools/bbmodel_convert.py.
    draw("cartridge_44", CARTRIDGE, 32)
    draw("bullet", BULLET, 32)
    draw("bullet_hole", BULLET_HOLE, 16)


if __name__ == "__main__":
    main()
