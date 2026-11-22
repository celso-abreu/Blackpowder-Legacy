#!/usr/bin/env python3
"""Convert the Winchester's Blockbench model (models/winchester_1873.bbmodel)
into Horizon's full item model JSON, the 6 standalone animated sub-models
(assets/blackpowder_legacy/models/special/winchester_*.json), and its 1024px
texture PNG.

Horizon's model format is vanilla's with one extension: an element's
`rotation` may be `{"origin": [...], "x": deg, "y": ry, "z": rz}` (Euler XYZ,
degrees) instead of vanilla's single `axis`/`angle` pair. That is what lets
this "free"-format Blockbench model (arbitrary angles) bake unmodified.

Constraints honoured here:
- element from/to must stay within [-16, 32] (vanilla model bounds) — the
  model is scaled down uniformly and recentered to fit;
- face UVs are converted from texture pixels (1024x1024) to vanilla's 0..16;
- `display` transforms are calibrated to 1:1 world scale (0.45 -> 1.34 blocks).

Run: python3 tools/bbmodel_convert.py
"""

import base64
import json
import pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
SOURCE = ROOT / "models" / "winchester_1873.bbmodel"
MODEL_OUT = ROOT / "assets" / "blackpowder_legacy" / "models" / "item" / "winchester_1873.json"
SPECIAL_DIR = ROOT / "assets" / "blackpowder_legacy" / "models" / "special"
TEXTURE_OUT = ROOT / "assets" / "blackpowder_legacy" / "textures" / "item" / "rifle_texture.png"
TEXTURE_ID = "blackpowder_legacy:item/rifle_texture"

# Uniform scale fitting the longest span (the barrel, Z) into the vanilla
# bounds with a small safety margin against float rounding at the edges.
SCALE = 0.48
BOUNDS = (-16.0, 32.0)

CARTRIDGE_ELEMENTS = [
    {
        "from": [7.71, 7.71, 8.6],
        "to": [8.29, 8.29, 8.72],
        "faces": {
            "north": {"uv": [8.55, 6.35, 8.7, 6.5], "texture": "#0"},
            "south": {"uv": [8.55, 6.35, 8.7, 6.5], "texture": "#0"},
            "east": {"uv": [8.5, 6.3, 8.75, 6.55], "texture": "#0"},
            "west": {"uv": [8.5, 6.3, 8.75, 6.55], "texture": "#0"},
            "up": {"uv": [8.5, 6.3, 8.75, 6.55], "texture": "#0"},
            "down": {"uv": [8.5, 6.3, 8.75, 6.55], "texture": "#0"},
        },
    },
    {
        "from": [7.75, 7.75, 7.54],
        "to": [8.25, 8.25, 8.6],
        "faces": {
            face: {"uv": [8.5, 6.3, 8.75, 6.55], "texture": "#0"}
            for face in ("north", "south", "east", "west", "up", "down")
        },
    },
    {
        "from": [7.78, 7.78, 7.1],
        "to": [8.22, 8.22, 7.54],
        "faces": {
            face: {"uv": [8.82, 0.25, 8.98, 0.65], "texture": "#0"}
            for face in ("north", "south", "east", "west", "up", "down")
        },
    },
]


def rounded(value: float) -> float:
    return round(value, 3)


def classify_part(name: str) -> str:
    if name.startswith("alavanca_"):
        return "lever"
    if name.startswith("cao"):
        return "hammer"
    if name.startswith("gatilho_"):
        return "trigger"
    if name.startswith("porta_"):
        return "gate"
    return "body"


def main() -> None:
    model = json.loads(SOURCE.read_text())
    elements = model["elements"]
    texture = model["textures"][0]
    tex_w = int(texture.get("uv_width") or texture.get("width"))
    tex_h = int(texture.get("uv_height") or texture.get("height"))

    # Model bounds, for centering.
    mins = [min(e["from"][i] for e in elements) for i in range(3)]
    maxs = [max(e["to"][i] for e in elements) for i in range(3)]
    center = [(mins[i] + maxs[i]) / 2 for i in range(3)]
    span = [(maxs[i] - mins[i]) * SCALE for i in range(3)]
    for axis in range(3):
        assert span[axis] <= BOUNDS[1] - BOUNDS[0], f"axis {axis} does not fit"

    # Target: model centred on [8, 8, 8] — vanilla's model-space centre —
    # except Z, where the barrel (starting at z=min, the muzzle) begins at the
    # lower bound so the whole length fits.
    offsets = [8.0 - center[0] * SCALE, 8.0 - center[1] * SCALE, BOUNDS[0] - mins[2] * SCALE]

    def point(p):
        return [rounded(p[i] * SCALE + offsets[i]) for i in range(3)]

    out_elements = []
    parts: dict[str, list[dict]] = {
        "body": [],
        "lever": [],
        "hammer": [],
        "trigger": [],
        "gate": [],
    }

    for e in elements:
        converted = {"from": point(e["from"]), "to": point(e["to"]), "faces": {}}
        rotation = e.get("rotation")
        if rotation and any(abs(a) > 1e-6 for a in rotation):
            converted["rotation"] = {
                "origin": point(e["origin"]),
                "x": rounded(rotation[0]),
                "y": rounded(rotation[1]),
                "z": rounded(rotation[2]),
            }
            if e.get("rescale"):
                converted["rotation"]["rescale"] = True
        for face_name, face in e.get("faces", {}).items():
            if face is None or face.get("texture") is None or face.get("uv") is None:
                continue
            uv = face["uv"]
            out_face = {
                "uv": [
                    rounded(uv[0] / tex_w * 16),
                    rounded(uv[1] / tex_h * 16),
                    rounded(uv[2] / tex_w * 16),
                    rounded(uv[3] / tex_h * 16),
                ],
                "texture": "#0",
            }
            if face.get("rotation"):
                out_face_rotation = int(face["rotation"])
                if out_face_rotation % 90 != 0:
                    raise ValueError(f"{e['name']}:{face_name} face rotation {out_face_rotation}")
                out_face["rotation"] = out_face_rotation
            converted["faces"][face_name] = out_face
        if converted["faces"]:
            out_elements.append(converted)
            parts[classify_part(e.get("name", ""))].append(converted)

    textures_map = {"0": TEXTURE_ID, "particle": TEXTURE_ID}

    item_model = {
        "credit": "Winchester 1873 3D model, converted from models/winchester_1873.bbmodel",
        "textures": textures_map,
        "elements": out_elements,
        "display": {
            "thirdperson_righthand": {
                "rotation": [5.73, -17.19, 0],
                "translation": [1.4, 0.75, -4.49],
                "scale": [0.45, 0.45, 0.45],
            },
            "thirdperson_lefthand": {
                "rotation": [5.73, 17.19, 0],
                "translation": [-1.4, 0.75, -4.49],
                "scale": [0.45, 0.45, 0.45],
            },
            "firstperson_righthand": {
                "rotation": [0, 0, 0],
                "translation": [0, 0.3, -4.75],
                "scale": [0.45, 0.45, 0.45],
            },
            "ground": {
                "rotation": [0, 0, 0],
                "translation": [0, 3, 0],
                "scale": [0.45, 0.45, 0.45],
            },
            "gui": {
                "rotation": [30, 225, 0],
                "translation": [0, 0, 0],
                "scale": [0.4, 0.4, 0.4],
            },
            "fixed": {
                "rotation": [0, 90, 0],
                "translation": [0, 0, 0],
                "scale": [0.4, 0.4, 0.4],
            },
        },
    }

    MODEL_OUT.parent.mkdir(parents=True, exist_ok=True)
    SPECIAL_DIR.mkdir(parents=True, exist_ok=True)
    TEXTURE_OUT.parent.mkdir(parents=True, exist_ok=True)

    MODEL_OUT.write_text(json.dumps(item_model, separators=(",", ":")))

    for part_name, part_elements in parts.items():
        special_out = SPECIAL_DIR / f"winchester_{part_name}.json"
        special_doc = {
            "credit": "Blackpowder Legacy — Winchester 1873 animated sub-model",
            "texture_size": [tex_w, tex_h],
            "textures": textures_map,
            "elements": part_elements,
        }
        special_out.write_text(json.dumps(special_doc, separators=(",", ":")))
        print(f"special-> {special_out} ({len(part_elements)} elements)")

    cartridge_out = SPECIAL_DIR / "winchester_cartridge.json"
    cartridge_doc = {
        "credit": "Blackpowder Legacy — .44-40 3D Cartridge",
        "texture_size": [tex_w, tex_h],
        "textures": textures_map,
        "elements": CARTRIDGE_ELEMENTS,
    }
    cartridge_out.write_text(json.dumps(cartridge_doc, separators=(",", ":")))
    print(f"special-> {cartridge_out} ({len(CARTRIDGE_ELEMENTS)} elements)")

    png = texture["source"].split(",", 1)[1]
    TEXTURE_OUT.write_bytes(base64.b64decode(png))
    print(f"elements: {len(out_elements)} (of {len(elements)})")
    print(f"model  -> {MODEL_OUT} ({MODEL_OUT.stat().st_size} bytes)")
    print(f"texture-> {TEXTURE_OUT} ({TEXTURE_OUT.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
