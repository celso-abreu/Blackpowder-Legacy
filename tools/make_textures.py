#!/usr/bin/env python3
"""Generate Just a Rifle's item, decal & package icon PNGs (deterministic).

Run:  python3 tools/make_textures.py
Writes PNGs into assets/just_a_rifle/textures/item/ and root icon.png using
Python's standard library only (no Pillow dependency required).
All PNGs are written with filter 0 (None) so tools/verify_assets.py can inspect
every scanline deterministically.
"""

from pathlib import Path
import json
import math
import struct
import zlib

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "assets" / "just_a_rifle" / "textures" / "item"

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


def _paeth(a: int, b: int, c: int) -> int:
    p = a + b - c
    pa = abs(p - a)
    pb = abs(p - b)
    pc = abs(p - c)
    if pa <= pb and pa <= pc:
        return a
    if pb <= pc:
        return b
    return c


def decode_png_rgba8(path: Path) -> tuple[int, int, bytearray]:
    raw = path.read_bytes()
    assert raw[:8] == b"\x89PNG\r\n\x1a\n"
    pos = 8
    width = height = 0
    idat = bytearray()
    while pos < len(raw):
        length = struct.unpack(">I", raw[pos : pos + 4])[0]
        ctype = raw[pos + 4 : pos + 8]
        chunk = raw[pos + 8 : pos + 8 + length]
        pos += 12 + length
        if ctype == b"IHDR":
            width, height, bd, ct, *_ = struct.unpack(">IIBBBBB", chunk)
            assert bd == 8 and ct == 6
        elif ctype == b"IDAT":
            idat.extend(chunk)
        elif ctype == b"IEND":
            break
    dec = zlib.decompress(bytes(idat))
    stride = width * 4
    pixels = bytearray(width * height * 4)
    prev = bytearray(stride)
    off = 0
    for y in range(height):
        ft = dec[off]
        row = bytearray(dec[off + 1 : off + 1 + stride])
        off += 1 + stride
        if ft == 1:
            for i in range(4, stride):
                row[i] = (row[i] + row[i - 4]) & 0xFF
        elif ft == 2:
            for i in range(stride):
                row[i] = (row[i] + prev[i]) & 0xFF
        elif ft == 3:
            for i in range(stride):
                left = row[i - 4] if i >= 4 else 0
                row[i] = (row[i] + ((left + prev[i]) >> 1)) & 0xFF
        elif ft == 4:
            for i in range(stride):
                left = row[i - 4] if i >= 4 else 0
                up = prev[i]
                up_left = prev[i - 4] if i >= 4 else 0
                row[i] = (row[i] + _paeth(left, up, up_left)) & 0xFF
        elif ft != 0:
            raise ValueError(f"Unsupported PNG filter {ft}")
        pixels[y * stride : (y + 1) * stride] = row
        prev = row
    return width, height, pixels


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


def _rot_x(p: tuple[float, float, float], deg: float) -> tuple[float, float, float]:
    rad = math.radians(deg)
    c, s = math.cos(rad), math.sin(rad)
    return (p[0], p[1] * c - p[2] * s, p[1] * s + p[2] * c)


def _rot_y(p: tuple[float, float, float], deg: float) -> tuple[float, float, float]:
    rad = math.radians(deg)
    c, s = math.cos(rad), math.sin(rad)
    return (p[0] * c + p[2] * s, p[1], -p[0] * s + p[2] * c)


def _rot_z(p: tuple[float, float, float], deg: float) -> tuple[float, float, float]:
    rad = math.radians(deg)
    c, s = math.cos(rad), math.sin(rad)
    return (p[0] * c - p[1] * s, p[0] * s + p[1] * c, p[2])


def _elem_rotate(
    p: tuple[float, float, float], rot: dict | None
) -> tuple[float, float, float]:
    if not rot:
        return p
    ox, oy, oz = rot["origin"]
    q = (p[0] - ox, p[1] - oy, p[2] - oz)
    if "axis" in rot:
        ang = float(rot.get("angle", 0.0))
        ax = rot["axis"]
        q = _rot_x(q, ang) if ax == "x" else (_rot_y(q, ang) if ax == "y" else _rot_z(q, ang))
    else:
        rx = float(rot.get("x", 0.0))
        ry = float(rot.get("y", 0.0))
        rz = float(rot.get("z", 0.0))
        q = _rot_z(_rot_y(_rot_x(q, rx), ry), rz)
    return (q[0] + ox, q[1] + oy, q[2] + oz)


FACE_CORNERS: dict[str, list[tuple[int, int, int]]] = {
    "north": [(1, 1, 0), (0, 1, 0), (0, 0, 0), (1, 0, 0)],
    "south": [(0, 1, 1), (1, 1, 1), (1, 0, 1), (0, 0, 1)],
    "west": [(0, 1, 0), (0, 1, 1), (0, 0, 1), (0, 0, 0)],
    "east": [(1, 1, 1), (1, 1, 0), (1, 0, 0), (1, 0, 1)],
    "up": [(0, 1, 0), (1, 1, 0), (1, 1, 1), (0, 1, 1)],
    "down": [(0, 0, 1), (1, 0, 1), (1, 0, 0), (0, 0, 0)],
}


def build_package_icon() -> None:
    """Renders root icon.png (256x256 square RGBA8, filter 0, <256 KiB):
    Western brass-framed frontier badge showcasing the textured 3D Winchester 1873
    alongside a crisp .44-40 brass/copper cartridge emblem.
    """
    size = 256
    ss = 2
    hi = size * ss  # 512x512 supersampled raster for crisp anti-aliased edges

    tex_w, tex_h, tex_px = decode_png_rgba8(OUT / "rifle_texture.png")
    model = json.loads(
        (ROOT / "assets" / "just_a_rifle" / "models" / "item" / "winchester_1873.json").read_text()
    )

    # 1. Paint 256x256 Western Frontier badge (binary corner cutout + brass bevels + warm glow).
    bg = bytearray(size * size * 4)
    chamfer = 14
    for y in range(size):
        for x in range(size):
            dx_edge = min(x, size - 1 - x)
            dy_edge = min(y, size - 1 - y)
            if dx_edge + dy_edge < chamfer:
                continue  # clean transparent corner chamfer (alpha = 0)

            d_border = min(dx_edge, dy_edge, (dx_edge + dy_edge - chamfer))
            # Normalized center offset for warm hearth glow
            nx = (x - 128.0) / 128.0
            ny = (y - 128.0) / 128.0
            rad2 = nx * nx + ny * ny

            if d_border < 3:
                # Outer dark gunmetal-bronze edge
                r, g, b = 34, 26, 18
            elif d_border < 9:
                # Polished western brass frame with top-left light bevel
                bevel = ((1.0 - x / 255.0) * 0.55 + (1.0 - y / 255.0) * 0.45)
                if d_border == 3 or d_border == 8:
                    r, g, b = 112, 82, 22
                else:
                    r = int(162 + 78 * bevel)
                    g = int(124 + 68 * bevel)
                    b = int(34 + 44 * bevel)
            elif d_border < 12:
                # Inner dark leather recess
                r, g, b = 20, 15, 12
            elif d_border == 12:
                # Subtle inner brass pinstripe
                r, g, b = 132, 98, 30
            else:
                # Warm frontier saloon-walnut / dusk vignette + subtle woodgrain + amber receiver halo
                glow = max(0.0, 1.0 - rad2 * 0.78) ** 1.25
                grain = math.sin(y * 0.42 + math.sin(x * 0.07) * 1.8) * 2.2
                diag_band = max(0.0, 1.0 - abs((nx + ny) * 0.95)) ** 1.6 * 32.0
                r = int(max(14, min(255, 20 + 46 * glow + diag_band + grain)))
                g = int(max(11, min(255, 15 + 31 * glow + diag_band * 0.72 + grain * 0.7)))
                b = int(max(9, min(255, 12 + 16 * glow + diag_band * 0.36 + grain * 0.4)))

            idx = (y * size + x) * 4
            bg[idx : idx + 4] = bytes((r, g, b, 255))

    # Stamp 4 brass corner rivets on the inner corners
    for rx, ry in ((20, 20), (size - 21, 20), (20, size - 21), (size - 21, size - 21)):
        for oy in range(-4, 5):
            for ox in range(-4, 5):
                d2 = ox * ox + oy * oy
                if d2 <= 16:
                    px, py = rx + ox, ry + oy
                    idx = (py * size + px) * 4
                    if d2 >= 11:
                        bg[idx : idx + 4] = bytes((78, 56, 15, 255))
                    elif ox <= 0 and oy <= 0:
                        bg[idx : idx + 4] = bytes((244, 214, 118, 255))
                    else:
                        bg[idx : idx + 4] = bytes((189, 146, 42, 255))

    # 2. Supersampled 512x512 Z-buffered rasterization of the 3D Winchester 1873!
    # View orientation: dramatic 3/4 side-profile diagonal (-38° tilt, barrel pointing upper-left
    # to lower-right / classic frontier showcase angle so receiver, lever loop, hammer, walnut
    # buttstock, and octagonal barrel fill the badge heroically).
    rifle_rgba = bytearray(hi * hi * 4)
    zbuf = [-1e9] * (hi * hi)

    def transform_model_pt(pt: tuple[float, float, float]) -> tuple[float, float, float]:
        # Center around receiver/forestock balance point [8.14, 8.95, 8.2]
        q = (pt[0] - 8.14, pt[1] - 8.95, pt[2] - 8.20)
        # Heroic upward-right showcase: walnut buttstock in bottom-left, octagonal barrel
        # rising to top-right, receiver & lever loop centered over warm amber glow!
        q = _rot_y(q, -104.0)
        q = _rot_x(q, 16.0)
        q = _rot_z(q, 37.0)
        scale = 11.5
        sx = hi * 0.495 + q[0] * scale
        sy = hi * 0.485 - q[1] * scale
        sz = q[2] * scale
        return (sx, sy, sz)

    light_dir = (0.36, 0.68, 0.63)

    def raster_tri(
        v0: tuple[float, float, float],
        v1: tuple[float, float, float],
        v2: tuple[float, float, float],
        uv0: tuple[float, float],
        uv1: tuple[float, float],
        uv2: tuple[float, float],
        shade: float,
    ) -> None:
        min_x = max(28, int(math.floor(min(v0[0], v1[0], v2[0]))))
        max_x = min(hi - 29, int(math.ceil(max(v0[0], v1[0], v2[0]))))
        min_y = max(28, int(math.floor(min(v0[1], v1[1], v2[1]))))
        max_y = min(hi - 29, int(math.ceil(max(v0[1], v1[1], v2[1]))))
        if min_x > max_x or min_y > max_y:
            return
        denom = (v1[1] - v2[1]) * (v0[0] - v2[0]) + (v2[0] - v1[0]) * (v0[1] - v2[1])
        if abs(denom) < 1e-6:
            return
        inv_d = 1.0 / denom
        for py in range(min_y, max_y + 1):
            cy = py + 0.5
            for px in range(min_x, max_x + 1):
                cx = px + 0.5
                w0 = ((v1[1] - v2[1]) * (cx - v2[0]) + (v2[0] - v1[0]) * (cy - v2[1])) * inv_d
                w1 = ((v2[1] - v0[1]) * (cx - v2[0]) + (v0[0] - v2[0]) * (cy - v2[1])) * inv_d
                w2 = 1.0 - w0 - w1
                if w0 < -1e-4 or w1 < -1e-4 or w2 < -1e-4:
                    continue
                z = w0 * v0[2] + w1 * v1[2] + w2 * v2[2]
                idx = py * hi + px
                if z <= zbuf[idx]:
                    continue
                u = (w0 * uv0[0] + w1 * uv1[0] + w2 * uv2[0]) / 16.0
                v = (w0 * uv0[1] + w1 * uv1[1] + w2 * uv2[1]) / 16.0
                tx = max(0, min(tex_w - 1, int(u * tex_w)))
                ty = max(0, min(tex_h - 1, int(v * tex_h)))
                t_off = (ty * tex_w + tx) * 4
                ta = tex_px[t_off + 3]
                if ta < 128:
                    continue
                zbuf[idx] = z
                # Boost contrast & warm specular sheen slightly so blued steel & walnut pop at icon sizes
                tr = min(255, int(tex_px[t_off] * shade * 1.18 + 8))
                tg = min(255, int(tex_px[t_off + 1] * shade * 1.15 + 6))
                tb = min(255, int(tex_px[t_off + 2] * shade * 1.12 + 5))
                c_off = idx * 4
                rifle_rgba[c_off : c_off + 4] = bytes((tr, tg, tb, 255))

    for elem in model["elements"]:
        f0 = elem["from"]
        t0 = elem["to"]
        rot = elem.get("rotation")
        for fname, fspec in elem["faces"].items():
            corners_local = [
                (
                    t0[0] if c[0] else f0[0],
                    t0[1] if c[1] else f0[1],
                    t0[2] if c[2] else f0[2],
                )
                for c in FACE_CORNERS[fname]
            ]
            pts3 = [transform_model_pt(_elem_rotate(p, rot)) for p in corners_local]
            # Screen-space normal for back-face culling + directional hemisphere lighting
            ux, uy, uz = (pts3[1][0] - pts3[0][0], pts3[1][1] - pts3[0][1], pts3[1][2] - pts3[0][2])
            vx, vy, vz = (pts3[2][0] - pts3[0][0], pts3[2][1] - pts3[0][1], pts3[2][2] - pts3[0][2])
            # Note: screen Y points down, so flip Y when computing outward camera-facing normal
            nx = (-uy) * vz - uz * (-vy)
            ny = uz * vx - ux * vz
            nz = ux * (-vy) - (-uy) * vx
            nlen = math.sqrt(nx * nx + ny * ny + nz * nz)
            if nlen < 1e-6:
                continue
            nx, ny, nz = nx / nlen, ny / nlen, nz / nlen
            if nz <= -0.05:
                continue
            ndotl = max(0.0, nx * light_dir[0] + ny * light_dir[1] + nz * light_dir[2])
            shade = 0.58 + 0.56 * ndotl

            u1, v1, u2, v2 = fspec["uv"]
            uvs = [(u1, v1), (u2, v1), (u2, v2), (u1, v2)]
            uv_rot = int(fspec.get("rotation", 0)) // 90
            if uv_rot:
                uvs = uvs[-uv_rot:] + uvs[:-uv_rot]

            raster_tri(pts3[0], pts3[1], pts3[2], uvs[0], uvs[1], uvs[2], shade)
            raster_tri(pts3[0], pts3[2], pts3[3], uvs[0], uvs[2], uvs[3], shade)

    # Downsample 512x512 rifle layer (2x2 box filter) + drop shadow + subtle dark outline onto 256x256 bg
    rifle_256 = bytearray(size * size * 4)
    for y in range(size):
        for x in range(size):
            r_sum = g_sum = b_sum = a_cnt = 0
            for sy in range(ss):
                for sx in range(ss):
                    off = ((y * ss + sy) * hi + (x * ss + sx)) * 4
                    if rifle_rgba[off + 3] > 0:
                        r_sum += rifle_rgba[off]
                        g_sum += rifle_rgba[off + 1]
                        b_sum += rifle_rgba[off + 2]
                        a_cnt += 1
            if a_cnt > 0:
                idx = (y * size + x) * 4
                rifle_256[idx] = r_sum // a_cnt
                rifle_256[idx + 1] = g_sum // a_cnt
                rifle_256[idx + 2] = b_sum // a_cnt
                rifle_256[idx + 3] = (a_cnt * 255) // (ss * ss)

    # Soft drop shadow under rifle (offset +4, +6)
    for y in range(14, size - 14):
        for x in range(14, size - 14):
            sx, sy = x - 4, y - 6
            if 0 <= sx < size and 0 <= sy < size:
                sa = rifle_256[(sy * size + sx) * 4 + 3]
                if sa > 0:
                    b_idx = (y * size + x) * 4
                    if bg[b_idx + 3] == 255:
                        k = 1.0 - 0.52 * (sa / 255.0)
                        bg[b_idx] = int(bg[b_idx] * k)
                        bg[b_idx + 1] = int(bg[b_idx + 1] * k)
                        bg[b_idx + 2] = int(bg[b_idx + 2] * k)

    # Crisp 1px dark rim around the rifle silhouette so it reads sharply even at 32x32 mod list size
    for y in range(14, size - 14):
        for x in range(14, size - 14):
            idx = (y * size + x) * 4
            if rifle_256[idx + 3] == 0:
                near = False
                for oy, ox in ((-1, 0), (1, 0), (0, -1), (0, 1)):
                    if rifle_256[((y + oy) * size + (x + ox)) * 4 + 3] >= 128:
                        near = True
                        break
                if near and bg[idx + 3] == 255:
                    bg[idx] = int(bg[idx] * 0.32)
                    bg[idx + 1] = int(bg[idx + 1] * 0.30)
                    bg[idx + 2] = int(bg[idx + 2] * 0.28)

    # Alpha-composite supersampled 3D Winchester 1873 over badge
    for y in range(size):
        for x in range(size):
            idx = (y * size + x) * 4
            ra = rifle_256[idx + 3]
            if ra > 0 and bg[idx + 3] == 255:
                a = ra / 255.0
                inv = 1.0 - a
                bg[idx] = int(rifle_256[idx] * a + bg[idx] * inv)
                bg[idx + 1] = int(rifle_256[idx + 1] * a + bg[idx + 1] * inv)
                bg[idx + 2] = int(rifle_256[idx + 2] * a + bg[idx + 2] * inv)

    # 3. Stamp a crisp 3x-upscaled .44-40 brass/copper cartridge in the lower-right badge corner!
    cart_scale = 3
    base_x = size - 20 - 32 * cart_scale
    base_y = size - 18 - 32 * cart_scale
    for cy, row in enumerate(CARTRIDGE):
        for cx, ch in enumerate(row):
            if ch == ".":
                continue
            cr, cg, cb, _ = PALETTE[ch]
            for sy in range(cart_scale):
                for sx in range(cart_scale):
                    px = base_x + cx * cart_scale + sx
                    py = base_y + cy * cart_scale + sy
                    # Subtle shadow first
                    sh_idx = ((py + 2) * size + (px + 2)) * 4
                    if 0 <= py + 2 < size and 0 <= px + 2 < size and bg[sh_idx + 3] == 255:
                        bg[sh_idx] = int(bg[sh_idx] * 0.65)
                        bg[sh_idx + 1] = int(bg[sh_idx + 1] * 0.65)
                        bg[sh_idx + 2] = int(bg[sh_idx + 2] * 0.65)
    for cy, row in enumerate(CARTRIDGE):
        for cx, ch in enumerate(row):
            if ch == ".":
                continue
            cr, cg, cb, _ = PALETTE[ch]
            for sy in range(cart_scale):
                for sx in range(cart_scale):
                    px = base_x + cx * cart_scale + sx
                    py = base_y + cy * cart_scale + sy
                    idx = (py * size + px) * 4
                    if bg[idx + 3] == 255:
                        bg[idx : idx + 4] = bytes((cr, cg, cb, 255))

    rows_bytes = [bytes(bg[y * size * 4 : (y + 1) * size * 4]) for y in range(size)]
    icon_path = ROOT / "icon.png"
    write_png_rgba(icon_path, size, size, rows_bytes)
    sz_kib = icon_path.stat().st_size / 1024.0
    print(f"wrote {icon_path} ({size}x{size}, {sz_kib:.1f} KiB)")


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    draw("cartridge_44", CARTRIDGE, 32)
    draw("bullet", BULLET, 32)
    draw("bullet_hole", BULLET_HOLE, 16)
    build_package_icon()


if __name__ == "__main__":
    main()
