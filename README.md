# Blackpowder Legacy

Blackpowder-era firearms for [Horizon](https://github.com/horizon-rust/horizon) (Minecraft: Java Edition 26.3 in Rust), as one WebAssembly `.hmod`.

**v0.8.0 — two-handed first-person rig, cocked-hammer sight alignment, 2-layer recoil & continuous reload.**

## How it plays

- Craft the rifle (iron, oak, a stick and a tripwire hook) and .44-40 cartridges (gunpowder + copper + iron nugget, 6 at a time).
- The tube magazine holds **8 rounds**; a brass bar under the icon tracks them, and the counter shows on the action bar.
- **Hold right click** to shoulder and aim: the cocked hammer clears the sight line and the iron sights slide dead-center onto the screen via FPS-independent second-order dynamics, the world FOV zooms (`0.67`), mouse sensitivity scales proportionally, and a subtle vignette frames the view. Both of the player's arms (`right_arm` on the grip, `left_arm` under the wooden forestock) move with the rifle. Aiming is suppressed while sprinting or reloading.
- **Left click** (aimed or hipfire while holding the rifle) fires one `.44-40` round (`15.0` b/t velocity, `0.06` spread in ADS / `0.04` crouched vs `1.50` hipfire / `2.20` moving). Each live shot applies **2-layer recoil** (camera pitch climb + `57%` smooth recovery + micro-jolt, plus rear-stock-pivoted 2nd-order gun kick) followed by the `10`-tick lever cycle (`120 RPM`). Clicking fire mid-reload immediately interrupts the reload when `>= 1` round is already in the tube.
- **R** (a real Controls keybind, remappable in **Options > Controls**) starts a **continuous per-cartridge reload loop** on a single press (`8t` open + `10t`/round + `6t` close) until `8 / 8` or interrupted by firing, sprinting, or switching slots. The left hand slides back to push each shell into the side loading gate (`porta_B`), without triggering vanilla's item re-equip bob.

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
- **Controls & Networking:** aiming is the item's use-over-time (`use_duration` > 0 on right click); `render_frame_pre` steps second-order springs per frame (`event.elapsed`), driving world FOV zoom (`compute_fov`), mouse turn scaling (`calculate_player_turn`), the vignette (`gui_render_post`), sight alignment (`render_hand`), and smooth camera recoil (`set_local_rotation`). Firing and reloading use bidirectional typed payloads (`RifleInput` client→server and `RifleSync` server→client) so recoil/lever animations only trigger on authoritative live shots, dry clicks give a subtle flinch, and `should_cause_reequip_animation` suppresses equip dips on ammo updates.
- **First-person arms:** `client_setup` bakes `minecraft:player#main` once and extracts `right_arm` (+ `right_sleeve`) and `left_arm` (+ `left_sleeve`) with zeroed local poses; `draw_arms` replicates `render_hand`'s exact pose matrix + `HAND_BASE` on `event.draw` with `[sxz, -sy, -sxz]` scaling so both hands follow ADS, recoil, lever action, and loading-gate shell insertion in lockstep.
- **Projectile:** `blackpowder_legacy:bullet` is a content-declared thrown type (snowball template) launched server-side with `host.spawn_projectile_from_rotation` at 6× throw power and tight spread. Bullet drop and damage falloff are future material.
- **Sounds:** vanilla events broadcast through `LevelRef::play_sound` (`firework_rocket.blast` for the shot, crossbow clicks for the lever and the last cartridge) — no chat spam. Custom `.ogg` gunshots replace them in a later version.
- **3D model:** `models/winchester_1873.bbmodel` (Blockbench "free" project, 336 elements, one 1024×1024 sheet) is converted by `tools/bbmodel_convert.py` into `assets/blackpowder_legacy/models/item/winchester_1873.json`. The converter scales the model ×0.48 into Horizon's [-16, 32] element bounds, divides the pixel UVs into the 0–16 space and keeps Blockbench's Euler-XYZ element rotations, which Horizon's model parser accepts natively. Run it again (`python3 tools/bbmodel_convert.py`) after editing the model in Blockbench.

## Roadmap

- v0.8: read the TAC (Timeless and Classics) decompiled source as design reference — recoil curve, ADS transition, reload state machine — and apply what fits.
- Later: Colt SAA revolver and blunderbuss (spread shot), misfires, per-part durability, a settings screen (zoom strength, vignette, keybinds).
