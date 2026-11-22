# Blackpowder Legacy

Blackpowder-era firearms for [Horizon](https://github.com/horizon-rust/horizon) (Minecraft: Java Edition 26.3 in Rust), as one WebAssembly `.hmod`.

**v0.7.1 — maintenance: the first-person arm holds the rifle, and the tree is tidied.**

## How it plays

- Craft the rifle (iron, oak, a stick and a tripwire hook) and .44-40 cartridges (gunpowder + copper + iron nugget, 6 at a time).
- The tube magazine holds **8 rounds**; a brass bar under the icon tracks them, and the counter shows on the action bar.
- **Hold right click** to shoulder and aim: the view zooms in, a light vignette frames it and the rifle slides onto the sights. The player's own arm is drawn holding the grip. Aiming for 0.4 s works the lever — the rifle is then ready to fire.
- **Left click while aiming** fires one round. The shot kicks, then the lever cycles again (`LEVER_TICKS` between shots, enforced server-side) before the next round.
- **R** loads one cartridge per press, `RELOAD_TICKS` apart, until the tube is full (remappable in **Options > Controls**, under Blackpowder Legacy). The gun dips as each round slides in.
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

## CI

Every push builds and checks the mod on GitHub Actions
(`.github/workflows/build-mod.yml`): `cargo horizon-mod build --wasm` then
`cargo horizon-mod check`, with the tool binary, the pinned SDK, the wasm
toolchain and the Cargo caches reused between runs. The checked
`blackpowder_legacy.hmod` is downloadable from the run's artifacts.

Both build steps run under `set -o pipefail`, so a failed compile fails the
job instead of being swallowed by `tee`.

## Layout

```text
horizon-mod.toml            # manifest: id, version, api 0.2, capabilities
Cargo.toml                  # wasm code crate + SDK pin
src/lib.rs                  # the mod: hooks, the typed fire/reload channel, the
                            #   Winchester's ItemPart and the first-person arm
data/blackpowder_legacy/
  horizon/item/             # winchester_1873, cartridge_44, bullet (item declarations)
  horizon/entity_type/      # bullet: thrown projectile (snowball template, 12 damage)
  horizon/creative_tab/     # the mod's creative tab
  recipe/                   # rifle + cartridge recipes
assets/blackpowder_legacy/
  items/                    # 26.3 item definitions (name -> model)
  models/item/              # item models; the rifle's is a 336-element 3D model
  textures/item/            # 16x16 pixel art + the rifle's 1024px sheet
  lang/                     # en_us, pt_br
models/winchester_1873.bbmodel  # Blockbench source of the rifle's 3D model
tools/
  make_textures.py          # regenerates the 16x16 item textures (python3 + Pillow)
  bbmodel_convert.py        # converts the .bbmodel into a Horizon item model + texture
  decompile/                # study tools for reference mods, not part of the build
```

`tools/decompile/` holds the Windows scripts used to read other mods' code for
design reference — see its own [README](tools/decompile/DECOMPILAR.md). Nothing
there is packaged into the `.hmod`.

## Design notes

- **Magazine storage:** the tube count lives in the stack's vanilla `damage` component (0–8). `max_damage = 100` keeps every write valid and the rifle unstackable; nothing ever breaks. The `item_bar` hook replaces the (misleading) vanilla damage bar with a brass ammo gauge.
- **Controls:** aiming is the item's use-over-time (`use_duration` > 0 on right click); the client flags it locally, so the zoom (`compute_fov_modifier`), the vignette (`gui_render_post`) and the sight alignment (`render_hand`) never wait on the network. Firing is `key_mapping_pre` cancelling the vanilla attack while aiming, plus a typed serverbound payload; reloading is a real Controls keybind. The server stays authoritative over the tube, the lever timing and the projectile.
- **First-person arm:** `client_setup` bakes `minecraft:player#main` once and keeps the tree; `draw_forearm` takes the right arm out of it and poses it onto the rifle's grip, so the hand is the player's own skin. Because it draws under `render_hand`'s pose, it follows recoil, the lever and the reload dip for free. The grip's position in view space comes from the rifle's own `firstperson_righthand` display transform — note vanilla writes that translation in sixteenths of a block.
- **Projectile:** `blackpowder_legacy:bullet` is a content-declared thrown type (snowball template) launched server-side with `host.spawn_projectile_from_rotation` at 6× throw power and tight spread. Bullet drop and damage falloff are future material.
- **Sounds:** vanilla events broadcast through `LevelRef::play_sound` (`firework_rocket.blast` for the shot, crossbow clicks for the lever and the last cartridge) — no chat spam. Custom `.ogg` gunshots replace them in a later version.
- **3D model:** `models/winchester_1873.bbmodel` (Blockbench "free" project, 336 elements, one 1024×1024 sheet) is converted by `tools/bbmodel_convert.py` into `assets/blackpowder_legacy/models/item/winchester_1873.json`. The converter scales the model ×0.48 into Horizon's [-16, 32] element bounds, divides the pixel UVs into the 0–16 space and keeps Blockbench's Euler-XYZ element rotations, which Horizon's model parser accepts natively. Run it again (`python3 tools/bbmodel_convert.py`) after editing the model in Blockbench.

## Roadmap

- v0.8: read the TAC (Timeless and Classics) decompiled source as design reference — recoil curve, ADS transition, reload state machine — and apply what fits.
- Later: Colt SAA revolver and blunderbuss (spread shot), misfires, per-part durability, a settings screen (zoom strength, vignette, keybinds).
