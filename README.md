# Blackpowder Legacy

Blackpowder-era firearms for [Horizon](https://github.com/horizon-rust/horizon) (Minecraft: Java Edition 26.3 in Rust), as one WebAssembly `.hmod`.

**v0.5 — the Winchester gets its true 3D model, converted from Blockbench.**

## How it plays

- Craft the rifle (iron, oak, a stick and a tripwire hook) and .44-40 cartridges (gunpowder + copper + iron nugget, 6 at a time).
- The tube magazine holds **8 rounds**; a brass bar under the icon tracks them, and the counter shows on the action bar.
- **Hold right click** to shoulder and aim: the view zooms in and a light vignette frames it. Aiming for 0.4 s works the lever — the rifle is then ready to fire.
- **Left click while aiming** fires one round. Work the lever again (keep holding right click) before the next shot.
- **R** loads one cartridge per press, until the tube is full (remappable in **Options > Controls**, under Blackpowder Legacy).
- Away from the aim the rifle never mines or punches; left click behaves normally.

## Build and test

You need Rust 1.95+, the Horizon launcher and `cargo-horizon-mod`:

```sh
cargo install --locked --git https://github.com/horizon-rust/horizon cargo-horizon-mod
cargo horizon-mod build --wasm          # fetches the pinned SDK into .horizon/, writes mod.wasm, packs target/horizon-mod/blackpowder_legacy.hmod
cargo horizon-mod check target/horizon-mod/blackpowder_legacy.hmod
cargo horizon-mod run                   # builds and starts the game with the mod on
```

Or copy the `.hmod` into the game folder's `horizon-mods/` and add `mod.blackpowder_legacy:true` to `horizon-mods.txt`.

The build this mod is made for is pinned in `Cargo.toml` (`[package.metadata.horizon] sdk`, Nightly 2026-11-20, `0a875da`). `cargo horizon-mod update` moves the pin.

## CI

Every push builds and checks the mod on GitHub Actions
(`.github/workflows/build-mod.yml`): `cargo horizon-mod build --wasm` then
`cargo horizon-mod check`, with the tool binary, the pinned SDK, the wasm
toolchain and the Cargo caches reused between runs. The checked
`blackpowder_legacy.hmod` is downloadable from the run's artifacts.

## Installing

This is a **wasm** mod: install the packaged `.hmod`, no build needed.

1. Download `blackpowder_legacy.hmod` from the [latest
   release](https://github.com/celso-abreu/Blackpowder-Legacy/releases/latest).
2. Drop it into the instance's **`horizon-mods/`** folder (from the launcher:
   open the instance and use **Open the instance folder**; create
   `horizon-mods` if missing).
3. Play — wasm mods need no native-code consent.

Note: the launcher's *Add a native mod from GitHub* dialog is for **native**
mods only; it refuses wasm packages (`not a native package`). Tagged releases
(`v*`, matching `version` in `horizon-mod.toml`) are built, checked and
published by `.github/workflows/release.yml`.

## Layout

```text
horizon-mod.toml            # manifest: id, version, api 0.2, capabilities
Cargo.toml                  # wasm code crate + SDK pin
src/lib.rs                  # the mod: input hooks, the typed fire/reload channel and the Winchester's ItemPart
data/blackpowder_legacy/
  horizon/item/             # winchester_1873, cartridge_44, bullet (item declarations)
  horizon/entity_type/      # bullet: thrown projectile (snowball template, 12 damage)
  horizon/creative_tab/     # the mod's creative tab
  recipe/                   # rifle + cartridge recipes
assets/blackpowder_legacy/
  items/ models/item/       # 26.3 item model definitions (the rifle's is a 336-element 3D model)
  textures/item/            # 16x16 pixel art (tools/make_textures.py) + the rifle's 1024px sheet
  lang/                     # en_us, pt_br
models/winchester_1873.bbmodel  # Blockbench source of the rifle's 3D model
tools/make_textures.py      # regenerates the 16x16 textures (python3 + Pillow)
tools/bbmodel_convert.py    # converts the .bbmodel into a Horizon item model + texture
```

## Design notes

- **Magazine storage:** the tube count lives in the stack's vanilla `damage` component (0–8). `max_damage = 100` keeps every write valid and the rifle unstackable; nothing ever breaks. The `item_bar` hook replaces the (misleading) vanilla damage bar with a brass ammo gauge.
- **Controls:** aiming is the item's use-over-time (`use_duration` > 0 on right click); the client flags it locally, so the fixed zoom (`compute_fov_modifier`) and the vignette (`gui_render_post`) never wait on the network. Firing is `key_mapping_pre` cancelling the vanilla attack while aiming, plus a typed serverbound payload; reloading is `input_key` on R. The server stays authoritative over the tube, the lever timing and the projectile.
- **Projectile:** `blackpowder_legacy:bullet` is a content-declared thrown type (snowball template) launched server-side with `host.spawn_projectile_from_rotation` at 6× throw power and tight spread. Bullet drop and damage falloff are future material.
- **Sounds:** vanilla events broadcast through `LevelRef::play_sound` (`firework_rocket.blast` for the shot, crossbow loading for the gate) — no chat spam. Custom `.ogg` gunshots replace them in a later version.
- **3D model:** `models/winchester_1873.bbmodel` (Blockbench "free" project, 336 elements, one 1024×1024 sheet) is converted by `tools/bbmodel_convert.py` into `assets/blackpowder_legacy/models/item/winchester_1873.json`. The converter scales the model ×0.48 into Horizon's [-16, 32] element bounds, divides the pixel UVs into the 0–16 space and keeps Blockbench's Euler-XYZ element rotations, which Horizon's model parser accepts natively. Run it again (`python3 tools/bbmodel_convert.py`) after editing the model in Blockbench.

## Roadmap

- Done in v0.5: a proper 3D model for the rifle (first/third person, ground, GUI). Still open: aim pose and recoil animation.
- v0.6: Colt SAA revolver and blunderbuss (spread shot), misfires, per-part durability, a settings screen (zoom strength, vignette, keybinds).
