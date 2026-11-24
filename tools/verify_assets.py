#!/usr/bin/env python3
"""Fast, zero-dependency integrity checks for Just a Rifle's manifests & assets.

Verifies:
1. `Cargo.toml`, `horizon-mod.toml`, and GitHub Actions workflows agree on the
   mod version, `just_a_rifle` namespace, and pinned `HORIZON_SDK_COMMIT`.
2. Every JSON under `assets/` and `data/` parses cleanly.
3. `en_us.json` and `pt_br.json` have identical key sets covering all subtitles,
   items, keybinds, and the creative tab.
4. Every `.ogg` referenced by `sounds.json` exists on disk.
5. `bullet_hole.png` has strictly binary alpha (`0` or `255`) so Horizon bakes
   `special/bullet_hole` on the cutout GPU-instanced path.
"""

from pathlib import Path
import json
import re
import struct
import zlib

ROOT = Path(__file__).resolve().parent.parent
MOD_ID = "just_a_rifle"


def parse_rgba8_png_alphas(path: Path) -> set[int]:
    raw = path.read_bytes()
    assert raw[:8] == b"\x89PNG\r\n\x1a\n", f"{path}: invalid PNG header"
    pos = 8
    width = height = 0
    idat = bytearray()
    while pos < len(raw):
        length = struct.unpack(">I", raw[pos : pos + 4])[0]
        ctype = raw[pos + 4 : pos + 8]
        chunk = raw[pos + 8 : pos + 8 + length]
        pos += 12 + length
        if ctype == b"IHDR":
            width, height, bit_depth, color_type, *_ = struct.unpack(">IIBBBBB", chunk)
            assert bit_depth == 8 and color_type == 6, f"{path}: expected 8-bit RGBA PNG"
        elif ctype == b"IDAT":
            idat.extend(chunk)
        elif ctype == b"IEND":
            break

    decompressed = zlib.decompress(bytes(idat))
    stride = width * 4
    alphas: set[int] = set()
    prev = bytearray(stride)
    offset = 0
    for _ in range(height):
        ftype = decompressed[offset]
        row = bytearray(decompressed[offset + 1 : offset + 1 + stride])
        offset += 1 + stride
        if ftype == 1:  # Sub
            for i in range(4, stride):
                row[i] = (row[i] + row[i - 4]) & 0xFF
        elif ftype == 2:  # Up
            for i in range(stride):
                row[i] = (row[i] + prev[i]) & 0xFF
        elif ftype != 0:
            raise AssertionError(f"{path}: unsupported PNG filter {ftype}")
        for x in range(width):
            alphas.add(row[x * 4 + 3])
        prev = row
    return alphas


def main() -> None:
    cargo_txt = (ROOT / "Cargo.toml").read_text()
    hmod_txt = (ROOT / "horizon-mod.toml").read_text()

    cargo_ver = re.search(r'^version\s*=\s*"([^"]+)"', cargo_txt, re.M)
    hmod_ver = re.search(r'^version\s*=\s*"([^"]+)"', hmod_txt, re.M)
    hmod_id = re.search(r'^id\s*=\s*"([^"]+)"', hmod_txt, re.M)
    sdk_pin = re.search(r'^sdk\s*=\s*"([0-9a-f]{40})"', cargo_txt, re.M)

    assert cargo_ver and hmod_ver and cargo_ver.group(1) == hmod_ver.group(1), (
        "Cargo.toml and horizon-mod.toml version mismatch"
    )
    assert hmod_id and hmod_id.group(1) == MOD_ID, f"Expected id = {MOD_ID}"
    assert sdk_pin, "Missing 40-hex [package.metadata.horizon] sdk pin in Cargo.toml"
    sdk_sha = sdk_pin.group(1)

    for wf_name in ("build-mod.yml", "release.yml"):
        wf_txt = (ROOT / ".github" / "workflows" / wf_name).read_text()
        wf_pin = re.search(r"HORIZON_SDK_COMMIT:\s*([0-9a-f]{40})", wf_txt)
        assert wf_pin and wf_pin.group(1) == sdk_sha, (
            f"{wf_name} HORIZON_SDK_COMMIT does not match Cargo.toml sdk ({sdk_sha})"
        )

    json_files = sorted(
        list((ROOT / "assets").rglob("*.json")) + list((ROOT / "data").rglob("*.json"))
    )
    assert json_files, "No asset/data JSON files found"
    for jf in json_files:
        raw_text = jf.read_text()
        assert "blackpowder_legacy" not in raw_text, f"Stale namespace in {jf}"
        json.loads(raw_text)

    en = json.loads((ROOT / "assets" / MOD_ID / "lang" / "en_us.json").read_text())
    pt = json.loads((ROOT / "assets" / MOD_ID / "lang" / "pt_br.json").read_text())
    assert set(en.keys()) == set(pt.keys()), (
        f"en_us / pt_br key mismatch: {set(en.keys()) ^ set(pt.keys())}"
    )

    sounds = json.loads((ROOT / "assets" / MOD_ID / "sounds.json").read_text())
    for event_name, spec in sounds.items():
        subtitle = spec["subtitle"]
        assert subtitle in en, f"{event_name}: missing subtitle {subtitle}"
        for entry in spec["sounds"]:
            ns, rel = entry["name"].split(":", 1)
            assert ns == MOD_ID, f"{event_name}: unexpected namespace {ns}"
            ogg = ROOT / "assets" / ns / "sounds" / f"{rel}.ogg"
            assert ogg.is_file(), f"{event_name}: missing {ogg}"

    tab = json.loads(
        (ROOT / "data" / MOD_ID / "horizon" / "creative_tab" / f"{MOD_ID}.json").read_text()
    )
    assert tab["title"] in en, f"Missing creative tab translation {tab['title']}"

    settings_rs = (ROOT / "src" / "settings.rs").read_text()
    key_consts = dict(
        re.findall(r'const\s+(KEY_[A-Z0-9_]+)\s*:\s*&str\s*=\s*"([^"]+)"', settings_rs)
    )
    assert len(key_consts) >= 25, f"Expected >=25 setting keys in settings.rs, got {len(key_consts)}"
    for cname, key_val in key_consts.items():
        lbl = f"config.{MOD_ID}.{key_val}"
        tip = f"config.{MOD_ID}.{key_val}.tooltip"
        assert lbl in en, f"{cname}: missing translation {lbl}"
        assert tip in en, f"{cname}: missing tooltip translation {tip}"

    for cname, opts_blob in re.findall(
        r"choice_opt\(\s*(KEY_[A-Z0-9_]+)\s*,\s*&\[([^\]]+)\]", settings_rs, re.S
    ):
        key_val = key_consts[cname]
        for suffix in re.findall(r'"([^"]+)"', opts_blob):
            opt_key = f"config.{MOD_ID}.{key_val}.{suffix}"
            assert opt_key in en, f"{cname}: missing choice option translation {opt_key}"

    adv_dir = ROOT / "data" / MOD_ID / "advancement"
    adv_files = sorted(adv_dir.rglob("*.json"))
    assert len(adv_files) >= 10, f"Expected >=10 advancement JSONs, found {len(adv_files)}"
    adv_ids = {
        f"{MOD_ID}:{af.relative_to(adv_dir).with_suffix('').as_posix()}" for af in adv_files
    }
    used_triggers: set[str] = set()
    for af in adv_files:
        adv = json.loads(af.read_text())
        assert adv.get("criteria"), f"{af}: missing criteria"
        for crit in adv["criteria"].values():
            used_triggers.add(crit["trigger"])
        parent = adv.get("parent")
        if parent is not None:
            assert parent == "minecraft:recipes/root" or parent in adv_ids, (
                f"{af}: unknown advancement parent {parent}"
            )
        disp = adv.get("display")
        if disp is not None:
            for field in ("title", "description"):
                tr = disp[field]["translate"]
                assert tr in en, f"{af}: missing advancement translation {tr}"

    constants_rs = (ROOT / "src" / "constants.rs").read_text()
    for trig_name, trig_id in re.findall(
        r'const\s+(TRIG_[A-Z0-9_]+)\s*:\s*&str\s*=\s*"([^"]+)"', constants_rs
    ):
        assert trig_id in used_triggers, (
            f"{trig_name} ({trig_id}) is not referenced by any advancement JSON"
        )

    alphas = parse_rgba8_png_alphas(
        ROOT / "assets" / MOD_ID / "textures" / "item" / "bullet_hole.png"
    )
    assert alphas <= {0, 255}, (
        f"bullet_hole.png must have binary alpha {{0, 255}} for cutout instancing, got {sorted(alphas)}"
    )

    print(
        f"OK: {MOD_ID} v{cargo_ver.group(1)} (sdk {sdk_sha[:7]}), "
        f"{len(json_files)} JSONs, {len(en)} lang keys, {len(sounds)} sound events, cutout decal verified."
    )


if __name__ == "__main__":
    main()
