# Blackpowder Legacy

Blackpowder-era firearms for [Horizon](https://github.com/horizon-rust/horizon) (Minecraft: Java Edition 26.3 in Rust), as one WebAssembly `.hmod`.

**v0.1 — Winchester 1873 lever-action rifle.**

## How it plays

- Craft the rifle (iron, oak, a stick and a tripwire hook) and .44-40 cartridges (gunpowder + copper + iron nugget, 6 at a time).
- The tube magazine holds **8 rounds**; the item's wear bar tracks them.
- **Hold right click with an empty tube** to work the loading gate: one cartridge every 0.4 s, with the counter on the action bar.
- **Hold right click with rounds loaded** to shoulder the rifle; **release** (after at least 0.4 s — the lever cycle) to fire.
- Releasing a loading use never fires: click-clack, no bang.

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
src/lib.rs                  # the Winchester's ItemPart (use_duration / on_use_tick / release_using)
data/blackpowder_legacy/
  horizon/item/             # winchester_1873, cartridge_44, bullet (item declarations)
  horizon/entity_type/      # bullet: thrown projectile (snowball template, 12 damage)
  horizon/creative_tab/     # the mod's creative tab
  recipe/                   # rifle + cartridge recipes
assets/blackpowder_legacy/
  items/ models/item/       # 26.3 item model definitions
  textures/item/            # 16x16 pixel art (tools/make_textures.py regenerates)
  lang/                     # en_us, pt_br
tools/make_textures.py      # regenerates the textures (python3 + Pillow)
```

## Design notes

- **Magazine storage:** the tube count lives in the stack's vanilla `damage` component (0–8). `max_damage = 100` keeps every write valid and the rifle unstackable; nothing ever breaks. A custom component replaces this in a later version, together with a proper ammo bar (`item_bar` hook).
- **Projectile:** `blackpowder_legacy:bullet` is a content-declared thrown type (snowball template) launched server-side with `host.spawn_projectile_from_rotation` at 6× throw power and tight spread. Bullet drop and damage falloff are v0.2 material.
- **Sounds:** vanilla events played through `entity.run_command` (`firework_rocket.blast` for the shot, crossbow loading for the gate). Custom `.ogg` gunshots replace them in a later version.

## Roadmap

- v0.2: proper ammo bar (`item_bar`), aim pose/animation (`get_arm_pose`, `use_animation`), muzzle smoke, custom sounds, top-off reload keybind, recoil.
- v0.3: Colt SAA revolver and blunderbuss (spread shot), misfires, per-part durability.
