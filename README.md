# Just a Rifle

Just one finely crafted lever-action rifle for [Horizon](https://github.com/horizon-rust/horizon) (Minecraft: Java Edition 26.3 in Rust) — no bloated gunpack, packaged as a single lightweight WebAssembly `.hmod`.

**v0.9.0 — multi-part animated Winchester 1873 (`body`, `lever`, `hammer`, `trigger`, `gate`, `cartridge`), terrain bullet-hole decals, 3D bouncing `.44-40` brass shell casings, real player skin (`slim` / `wide` + 3D sleeves), clean ADS iron sights (HUD crosshair hidden), per-round side-gate reload, weapon inspect (`I`), diagonal 3rd-person back-sling (`F`), `32×32` `.44-40` art, and CC0 custom `.ogg` firearm sounds.**

## How it plays

- Craft the **Winchester 1873** (iron ingots, oak planks, a stick, and a tripwire hook) and **.44-40 Cartridges** (gunpowder + copper ingot + iron nugget, 6 at a time).
- The tubular magazine holds **8 rounds**; a brass bar under the item icon and a dedicated bottom-right HUD counter track loaded vs. reserve `.44-40` rounds.
- **Hold right click** to shoulder and aim down sights: the vanilla HUD crosshair (`minecraft:crosshair`) disappears automatically, the cocked hammer clears the sight line, and the rear notch + front blade (`alca_entalhe_base` + `massa_lamina`) slide dead-center onto the screen via FPS-independent second-order dynamics while FOV zooms (`0.67`), mouse turn sensitivity scales proportionally, and a subtle vignette frames the view. Aiming is suppressed while sprinting or reloading.
- **Left click** (aimed or hipfire while holding the rifle in the main hand) fires one `.44-40` round (`15.0` b/t velocity, `0.06` spread in ADS / `0.04` crouched vs. `1.50` hipfire / `2.20` moving). Each live shot drives **articulated sub-model animations** (trigger pull, hammer strike, `+48°` lever throw, and hammer re-cocking), **2-layer recoil** (camera pitch climb + `57%` smooth recovery + micro-jolt, plus rear-stock-pivoted 2nd-order gun kick), barrel-tip muzzle smoke/flame particles, **3D spent `.44-40` brass shell casings** that pop out of the top receiver port (`trava_dorsal`), tumble, bounce off terrain/slabs/stairs with a metallic clink, and rest on the ground, **world-lit pixel-art bullet-hole decals** on terrain blocks (auto-cleared when the block is broken), and layered CC0 gunshot + mechanical lever audio (`120 RPM`). Dry-firing on an empty chamber drops the hammer and keeps it uncocked until reloaded.
- **R** (remappable in **Options > Controls**) starts a **continuous per-cartridge reload loop** on a single press: tilts the receiver to expose `porta_B`, depresses the side loading gate (`-23°`) as each 3D `.44-40` brass/copper cartridge slides in with the left hand, and runs a full lever rack (`ReloadEndEmpty`) when reloading from `0/8` to chamber the first round and re-cock the hammer. Clicking fire mid-reload immediately interrupts the reload when `>= 1` round is already in the tube.
- **I** (remappable in **Options > Controls**) plays a 3.4-second **weapon inspection** sequence showing both sides of the receiver, walnut stock, and brass carrier when idle in hipfire.
- **F** (swap to off-hand) **slings the rifle diagonally across the player's back** in 3rd-person (`44°` barrel-up / stock-down, locked to the `Body` torso pose through walking, sneaking, and armor), relaxes the off-hand arm (`ArmPose::Empty`), and prevents accidental off-hand ADS right-clicks.

## Build and test

You need Rust 1.95+, the Horizon launcher, and `cargo-horizon-mod`:

```sh
cargo install --locked --git https://github.com/horizon-rust/horizon --rev 2b77287de099c9316278ba7ac4d83590fdb7ef30 cargo-horizon-mod
cargo horizon-mod build --wasm          # fetches the pinned SDK into .horizon/, writes mod.wasm, packs target/horizon-mod/just_a_rifle.hmod
cargo horizon-mod check target/horizon-mod/just_a_rifle.hmod
cargo horizon-mod run                   # builds and starts the game with the mod on
```

Or copy the `.hmod` into the game folder's `horizon-mods/` and add `mod.just_a_rifle:true` to `horizon-mods.txt`.

The Horizon build this mod targets is pinned in `Cargo.toml` (`[package.metadata.horizon] sdk`, Nightly 2026-11-22, `2b77287`). `cargo horizon-mod update` moves the pin.

## Installing

This is a **wasm** mod: install the packaged `.hmod`, no build needed.

1. Download `just_a_rifle.hmod` from the [latest release](https://github.com/celso-abreu/Blackpowder-Legacy/releases/latest).
2. Drop it into the instance's **`horizon-mods/`** folder (from the launcher: open the instance and use **Open the instance folder**; create `horizon-mods` if missing).
3. Play — wasm mods need no native-code consent.

Note: the launcher's *Add a native mod from GitHub* dialog is for **native** mods only; it refuses wasm packages (`not a native package`). Tagged releases (`v*`, matching `version` in `horizon-mod.toml`) are built, checked, and published by `.github/workflows/release.yml`.

## CI

Every push builds and checks the mod on GitHub Actions (`.github/workflows/build-mod.yml`): `cargo horizon-mod build --wasm` then `cargo horizon-mod check`, with the tool binary, the pinned SDK, the wasm toolchain, and the Cargo caches reused between runs. The checked `just_a_rifle.hmod` is downloadable from the run's artifacts.

Both `build-mod.yml` and `release.yml` run build/check steps under `set -o pipefail`, so a failed compile or registry check never passes silently through `tee`.

## Layout

```text
horizon-mod.toml            # manifest: id = "just_a_rifle", version 0.9.0, api 0.2, capabilities
Cargo.toml                  # wasm code crate + pinned Horizon SDK (2b77287)
SOUND-CREDITS.md            # CC0 / Public Domain attribution for all 9 .ogg audio files
src/lib.rs                  # WASM mod: articulated sub-model rig, TAC state machine,
                            #   player skin resolver, 2-layer recoil, ADS crosshair hide,
                            #   3rd-person diagonal back-sling & server-authoritative ammo
data/just_a_rifle/
  horizon/item/             # winchester_1873, cartridge_44, bullet (item declarations)
  horizon/entity_type/      # bullet: thrown projectile (snowball template, 12 damage)
  horizon/creative_tab/     # the mod's creative tab ("Just a Rifle")
  recipe/                   # rifle + cartridge recipes
assets/just_a_rifle/
  items/                    # 26.3 item definitions (selects empty model in player off-hand)
  models/item/              # full 336-element item model (GUI, ground, 3rd-person)
  models/special/           # 8 standalone sub-models: body, lever, hammer, trigger,
                            #   gate, cartridge, spent casing, and terrain bullet-hole decal
  textures/item/            # 32x32 cartridge & bullet, 16x16 cutout bullet hole + 1024x1024 rifle sheet
  sounds/ + sounds.json     # 9 CC0 custom .ogg firearm & Foley sound events + subtitles
  lang/                     # en_us, pt_br (items, keybinds, full audio subtitles)
models/winchester_1873.bbmodel  # Blockbench source of the rifle's 3D model
tools/
  make_textures.py          # deterministic pure-Python generator for item & decal PNGs
  bbmodel_convert.py        # converts the .bbmodel into item/winchester_1873.json,
                            #   models/special/winchester_*.json, and rifle_texture.png
  decompile/                # local study scripts for reference mods, not packaged
```

`tools/decompile/` holds local helper scripts used to study reference mods' mechanics — see [DECOMPILAR.md](tools/decompile/DECOMPILAR.md). Nothing under `models/` or `tools/` is packaged into the `.hmod`.

## Design notes

- **Articulated multi-part rig (`models/special/`):** `Hooks::standalone_models` registers standalone sub-models (`body`, `lever`, `hammer`, `trigger`, `gate`, `cartridge`, `casing`, `bullet_hole`) baked into `ModelRef` handles in `model_baking_completed`. In 1st person (`render_hand`) and when slung on the back (`render_player_post`), `draw_rifle_assembly` renders each mechanical sub-group around its exact Blockbench pivot (`LEVER_PIVOT_TEX`, `HAMMER_PIVOT_TEX`, `TRIGGER_PIVOT_TEX`, `GATE_PIVOT_TEX`) while keeping the monolithic `models/item/winchester_1873.json` for GUI, ground drops, and 3rd-person main-hand holds.
- **Real player skin & slim/wide arms:** `ServerHost::profile_by_uuid` decodes Mojang's Base64 `textures` property into Horizon's `"skins/" + sha1_utf16le(url_hash)` sprite key + `slim` (`3×12×4`) / `wide` (`4×12×4`) arm geometry (synced via `RifleSkinSync`), with deterministic frame-0 fallback to Horizon's 18 modern default skins (`default_skin_for_uuid`) and 3D outer sleeve layers (`CubeDeformation(0.25)`).
- **Controls, HUD & Networking:** aiming uses `use_duration` on main-hand right click (`ItemPart::use_item` rejects off-hand ADS), `Hooks::gui_layer_pre` hides `minecraft:crosshair` while aiming, and `render_frame_pre` steps FPS-independent second-order springs driving FOV (`compute_fov`), mouse turn scaling (`calculate_player_turn`), vignette + ammo HUD (`gui_render_post`), and smooth camera recoil (`set_local_rotation`). Firing, reloading, and inspecting communicate over typed channels (`RifleInput`, `RifleSync`, `RifleSkinSync`) with `should_cause_reequip_animation` suppressing equip dips on magazine updates.
- **3rd-person diagonal back-sling:** `assets/just_a_rifle/items/winchester_1873.json` uses a `minecraft:select` `display_context` switch to suppress vanilla's held off-hand model on players while `Hooks::get_arm_pose` relaxes the off-hand arm and `Hooks::render_player_post` mounts the rifle at `44°` across the player's `HumanoidPart::Body` matrix (adapting depth automatically when chest armor/elytra is worn and mirroring cleanly for left-handed players).
- **Custom CC0 audio:** 9 `.ogg` sound assets (`winchester_fire`, `winchester_thump`, `winchester_lever`, `winchester_cock`, `winchester_reload_intro`, `winchester_reload_insert`, `winchester_reload_end`, `winchester_draw`, `winchester_inspect`) with localized `pt_br` and `en_us` accessibility subtitles (see [SOUND-CREDITS.md](SOUND-CREDITS.md)).
- **Terrain bullet decals & 3D spent brass casings:** `Hooks::hears_projectile_impacts` + `Hooks::projectile_impact` detect `.44-40` terrain hits (`event.entity.is_none()`), spawn impact dust/smoke + stone crack audio, and broadcast `BulletImpactSync` (compact `0.1 mm` block-relative `VarInt` offsets) to nearby players. On the client, `Hooks::submit_custom_geometry` renders up to `96` world-lit cutout `special/bullet_hole` decals (sampling `LightLayer::Block` / `LightLayer::Sky` and pruning immediately when `host.block_collision(block)` empties) and up to `48` tumbling/bouncing `special/winchester_casing` `.44-40` shells (colliding against full blocks, slabs, stairs, and carpets via `host.block_collision`, clinking via `item.winchester_shell_drop`, and settling flat on the surface) in just two GPU-instanced batches (`WorldDrawContext::draw_model_instances`).

## Future polish

- Dynamic 3rd-person hip/ADS arm poses, structure chest loot tables, advancements, and optional client settings.
