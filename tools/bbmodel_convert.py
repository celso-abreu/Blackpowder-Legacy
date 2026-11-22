#!/usr/bin/env python3
"""Convert the Winchester's Blockbench model (models/winchester_1873.bbmodel)
into a Horizon item model JSON + its texture PNG.

Horizon's model format is vanilla's with one extension: an element's
`rotation` may be `{"origin": [...], "x": deg, "y": ry, "z": rz}` (Euler XYZ,
degrees) instead of vanilla's single `axis`/`angle` pair. That is what lets
this "free"-format Blockbench model (arbitrary angles) bake unmodified.

Constraints honoured here:
- element from/to must stay within [-16, 32] (vanilla model bounds) — the
  model is scaled down uniformly and recentered to fit;
- face UVs are converted from texture pixels (1024x1024) to vanilla's 0..16;
- `display` transforms are hand-tuned (translation clamps to ±5, scale ±4).

Run: python3 tools/bbmodel_convert.py
"""

import base64
import json
import math
import pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
SOURCE = ROOT / "models" / "winchester_1873.bbmodel"
MODEL_OUT = ROOT / "assets" / "blackpowder_legacy" / "models" / "item" / "winchester_1873.json"
TEXTURE_OUT = ROOT / "assets" / "blackpowder_legacy" / "textures" / "item" / "rifle_texture.png"
TEXTURE_ID = "blackpowder_legacy:item/rifle_texture"

# Uniform scale fitting the longest span (the barrel, Z) into the vanilla
# bounds with a small safety margin against float rounding at the edges.
SCALE = 0.48
BOUNDS = (-16.0, 32.0)


def rounded(value: float) -> float:
    return round(value, 3)


def main() -> None:
    model = json.loads(SOURCE.read_text())
    elements = model["elements"]
    texture = model["textures"][0]
    tex_w = texture.get("uv_width") or texture.get("width")
    tex_h = texture.get("uv_height") or texture.get("height")

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

    item_model = {
        "credit": "Winchester 1873 3D model, converted from models/winchester_1873.bbmodel",
        "textures": {"0": TEXTURE_ID, "particle": TEXTURE_ID},
        "elements": out_elements,
        # Display poses, derived from Horizon's render chain (vanilla-faithful):
        # third person applies arm * rotX(-90) * rotY(180) * T(1,2,-10)/16
        # (mobs/render.rs hand_base), which maps the item axes to
        # +X:character-right, +Y:forward, +Z:up — so rotation [90,0,0] turns
        # the barrel (-Z) forward and the top (+Y) up, and the translation
        # brings the grip (the stock wrist, ~(8, 7.3, 18.6) in model units)
        # onto the hand anchor. First person is plain view space (-Z
        # forward): identity rotation, translation pulls the grip to the
        # hand. Left hands inherit the right's entry and mirror it.
        "display": {
            "thirdperson_righthand": {"rotation": [90, 0, 0], "translation": [0, 4.75, 0.3], "scale": [0.45, 0.45, 0.45]},
            "firstperson_righthand": {"rotation": [0, 0, 0], "translation": [0, 0.3, -4.75], "scale": [0.45, 0.45, 0.45]},
            "ground": {"rotation": [0, 0, 0], "translation": [0, 2, 0], "scale": [0.25, 0.25, 0.25]},
            "gui": {"rotation": [30, 225, 0], "translation": [0, 0, 0], "scale": [0.4, 0.4, 0.4]},
            "fixed": {"rotation": [0, 90, 0], "translation": [0, 0, 0], "scale": [0.3, 0.3, 0.3]},
        },
    }

    MODEL_OUT.write_text(json.dumps(item_model, separators=(",", ":")))
    png = texture["source"].split(",", 1)[1]
    TEXTURE_OUT.write_bytes(base64.b64decode(png))
    print(f"elements: {len(out_elements)} (of {len(elements)})")
    print(f"model  -> {MODEL_OUT} ({MODEL_OUT.stat().st_size} bytes)")
    print(f"texture-> {TEXTURE_OUT} ({TEXTURE_OUT.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
