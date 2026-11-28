#!/usr/bin/env python3
"""Fast, zero-dependency integrity checks for Just a Rifle's manifests, docs & assets.

Verifies:
1. `Cargo.toml`, `horizon-mod.toml`, `README.md`, `CHANGELOG.md`, and GitHub Actions
   workflows agree on the mod version, `just_a_rifle` namespace, and `HORIZON_SDK_COMMIT`.
2. Every JSON under `assets/` and `data/` parses cleanly without stale namespaces.
3. All language files (`en_us.json`, `pt_br.json`, `pt_pt.json`) share identical key sets
   and `%s` format placeholder counts, covering all items, tooltips, subtitles, settings,
   and advancements.
4. Every `.ogg` in `assets/just_a_rifle/sounds/` is `48 kHz`, Mono when 3D-attenuated,
   referenced in `sounds.json`, and credited in `SOUND-CREDITS.md`.
5. `README.md` Layout accurately matches `src/*.rs` and `tools/*.py` on disk.
6. `bullet_hole.png` and `icon.png` have strictly binary alpha (`0` or `255`), and GUI
   3D LOD models stay within fast BSP element bounds.
"""

from pathlib import Path
import json
import re
import struct
import zlib

ROOT = Path(__file__).resolve().parent.parent
MOD_ID = "just_a_rifle"


def parse_rgba8_png_info(path: Path) -> tuple[int, int, set[int]]:
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
    return width, height, alphas


def main() -> None:
    cargo_txt = (ROOT / "Cargo.toml").read_text()
    hmod_txt = (ROOT / "horizon-mod.toml").read_text()
    readme_txt = (ROOT / "README.md").read_text()
    changelog_txt = (ROOT / "CHANGELOG.md").read_text()
    credits_txt = (ROOT / "SOUND-CREDITS.md").read_text()

    cargo_ver = re.search(r'^version\s*=\s*"([^"]+)"', cargo_txt, re.M)
    hmod_ver = re.search(r'^version\s*=\s*"([^"]+)"', hmod_txt, re.M)
    hmod_id = re.search(r'^id\s*=\s*"([^"]+)"', hmod_txt, re.M)
    sdk_pin = re.search(r'^sdk\s*=\s*"([0-9a-f]{40})"', cargo_txt, re.M)

    assert cargo_ver and hmod_ver and cargo_ver.group(1) == hmod_ver.group(1), (
        "Cargo.toml and horizon-mod.toml version mismatch"
    )
    ver = cargo_ver.group(1)
    assert f"v{ver}" in readme_txt, f"README.md does not document current version v{ver}"
    assert f"## `v{ver}`" in changelog_txt, (
        f"CHANGELOG.md missing release section ## `v{ver}`"
    )
    assert hmod_id and hmod_id.group(1) == MOD_ID, f"Expected id = {MOD_ID}"
    for req_field in ("homepage", "issues", "credits"):
        assert re.search(rf'^{req_field}\s*=\s*"[^"]+"', hmod_txt, re.M), (
            f"horizon-mod.toml missing {req_field}"
        )
    assert sdk_pin, "Missing 40-hex [package.metadata.horizon] sdk pin in Cargo.toml"
    sdk_sha = sdk_pin.group(1)

    for wf_name in ("build-mod.yml", "release.yml"):
        wf_txt = (ROOT / ".github" / "workflows" / wf_name).read_text()
        wf_pin = re.search(r"HORIZON_SDK_COMMIT:\s*([0-9a-f]{40})", wf_txt)
        assert wf_pin and wf_pin.group(1) == sdk_sha, (
            f"{wf_name} HORIZON_SDK_COMMIT does not match Cargo.toml sdk ({sdk_sha})"
        )

    # Verify README.md Layout lists exact `src/*.rs` and `tools/*.py` files on disk.
    disk_rs = {p.name for p in (ROOT / "src").glob("*.rs")}
    readme_rs = set(re.findall(r"^\s+([a-z0-9_]+\.rs)\b", readme_txt, re.M))
    assert disk_rs == readme_rs, (
        f"README.md src/*.rs drift: disk={sorted(disk_rs)} vs readme={sorted(readme_rs)}"
    )
    disk_py = {p.name for p in (ROOT / "tools").glob("*.py")}
    readme_py = set(re.findall(r"^\s+([a-z0-9_]+\.py)\b", readme_txt, re.M))
    assert disk_py == readme_py, (
        f"README.md tools/*.py drift: disk={sorted(disk_py)} vs readme={sorted(readme_py)}"
    )

    json_files = sorted(
        list((ROOT / "assets").rglob("*.json")) + list((ROOT / "data").rglob("*.json"))
    )
    assert json_files, "No asset/data JSON files found"
    for jf in json_files:
        raw_text = jf.read_text()
        assert "blackpowder_legacy" not in raw_text, f"Stale namespace in {jf}"
        json.loads(raw_text)

    lang_dir = ROOT / "assets" / MOD_ID / "lang"
    lang_files = sorted(lang_dir.glob("*.json"))
    assert len(lang_files) >= 3, f"Expected >=3 language files, found {len(lang_files)}"
    en = json.loads((lang_dir / "en_us.json").read_text())
    for lf in lang_files:
        data = json.loads(lf.read_text())
        assert set(data.keys()) == set(en.keys()), (
            f"{lf.name} / en_us.json key mismatch: {set(data.keys()) ^ set(en.keys())}"
        )
        for k, en_val in en.items():
            assert data[k].count("%s") == en_val.count("%s"), (
                f"{lf.name} key {k!r} %s placeholder mismatch vs en_us.json"
            )

    sounds = json.loads((ROOT / "assets" / MOD_ID / "sounds.json").read_text())
    referenced_oggs: set[Path] = set()
    for event_name, spec in sounds.items():
        subtitle = spec["subtitle"]
        assert subtitle in en, f"{event_name}: missing subtitle {subtitle}"
        for entry in spec["sounds"]:
            ns, rel = entry["name"].split(":", 1)
            assert ns == MOD_ID, f"{event_name}: unexpected namespace {ns}"
            ogg = ROOT / "assets" / ns / "sounds" / f"{rel}.ogg"
            assert ogg.is_file(), f"{event_name}: missing {ogg}"
            referenced_oggs.add(ogg.resolve())
            ogg_bytes = ogg.read_bytes()
            vhdr = ogg_bytes.find(b"\x01vorbis")
            assert vhdr >= 0, f"{ogg}: missing Vorbis identification header"
            _, v_ch, v_rate = struct.unpack("<IBI", ogg_bytes[vhdr + 7 : vhdr + 16])
            assert v_rate == 48_000, f"{ogg}: expected 48000 Hz sample rate, got {v_rate} Hz"
            if "attenuation_distance" in entry:
                assert v_ch == 1, (
                    f"{event_name} ({ogg.name}): positional 3D sound must be Mono (CH=1) for OpenAL attenuation, got CH={v_ch}"
                )

    disk_oggs = {p.resolve() for p in (ROOT / "assets" / MOD_ID / "sounds").rglob("*.ogg")}
    orphan_oggs = disk_oggs - referenced_oggs
    assert not orphan_oggs, (
        f"Unreferenced .ogg files in sounds/: {sorted(p.name for p in orphan_oggs)}"
    )
    disk_ogg_names = {p.name for p in disk_oggs}
    credited_ogg_names = set(re.findall(r"`([a-z0-9_]+\.ogg)`", credits_txt))
    assert disk_ogg_names == credited_ogg_names, (
        f"SOUND-CREDITS.md .ogg mismatch: missing={sorted(disk_ogg_names - credited_ogg_names)}, extra={sorted(credited_ogg_names - disk_ogg_names)}"
    )

    tab = json.loads(
        (ROOT / "data" / MOD_ID / "horizon" / "creative_tab" / f"{MOD_ID}.json").read_text()
    )
    assert tab["title"] in en, f"Missing creative tab translation {tab['title']}"

    settings_rs = (ROOT / "src" / "settings.rs").read_text()
    key_consts = dict(
        re.findall(r'const\s+(KEY_[A-Z0-9_]+)\s*:\s*&str\s*=\s*"([^"]+)"', settings_rs)
    )
    assert len(key_consts) >= 25, (
        f"Expected >=25 setting keys in settings.rs, got {len(key_consts)}"
    )
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

    _, _, alphas = parse_rgba8_png_info(
        ROOT / "assets" / MOD_ID / "textures" / "item" / "bullet_hole.png"
    )
    assert alphas <= {0, 255}, (
        f"bullet_hole.png must have binary alpha {{0, 255}} for cutout instancing, got {sorted(alphas)}"
    )

    icon_path = ROOT / "icon.png"
    assert icon_path.is_file(), "Missing package root icon.png"
    icon_bytes = icon_path.stat().st_size
    assert 0 < icon_bytes <= 262_144, f"icon.png exceeds 256 KiB limit ({icon_bytes} bytes)"
    iw, ih, icon_alphas = parse_rgba8_png_info(icon_path)
    assert iw == ih and 1 <= iw <= 512, f"icon.png must be square 1..=512 px, got {iw}x{ih}"
    assert icon_alphas <= {0, 255}, (
        f"icon.png expected crisp cutout corners, got {sorted(icon_alphas)}"
    )

    all_rs = "\n".join(p.read_text() for p in (ROOT / "src").glob("*.rs"))
    for tkey in re.findall(r'"(tooltip\.just_a_rifle\.[^"]+)"', all_rs):
        assert tkey in en, f"src/*.rs references missing tooltip translation {tkey}"

    # Guard against GUI 3D BSP polygon explosion: `items/winchester_1873.json` must route `gui`
    # to `item/winchester_1873_lod`, and both 3D LOD meshes must stay compact (< 60 elements).
    item_def = json.loads(
        (ROOT / "assets" / MOD_ID / "items" / "winchester_1873.json").read_text()
    )
    top_model = item_def["model"]
    assert (
        top_model.get("property") == "minecraft:display_context"
        and any(
            "gui" in c.get("when", [])
            and c.get("model", {}).get("model") == f"{MOD_ID}:item/winchester_1873_lod"
            for c in top_model.get("cases", [])
        )
    ), "items/winchester_1873.json must route GUI display_context to winchester_1873_lod"

    for rel_lod in ("item/winchester_1873_lod.json", "special/winchester_body_lod.json"):
        lod_json = json.loads((ROOT / "assets" / MOD_ID / "models" / rel_lod).read_text())
        assert 20 <= len(lod_json["elements"]) <= 60, (
            f"{rel_lod}: expected 20..=60 elements to avoid GUI BSP splitting, got {len(lod_json['elements'])}"
        )

    print(
        f"OK: {MOD_ID} v{ver} (sdk {sdk_sha[:7]}), "
        f"{len(json_files)} JSONs, {len(lang_files)} langs ({len(en)} keys each), "
        f"{len(sounds)} sound events ({len(disk_oggs)} .ogg credited), "
        f"{len(disk_rs)} Rust modules, 3D LOD & cutout decal verified."
    )


if __name__ == "__main__":
    main()
