# Just a Rifle

Just one finely crafted lever-action rifle for [Horizon](https://github.com/horizon-rust/horizon) (Minecraft: Java Edition 26.3 in Rust) — no bloated gunpack, packaged as a single lightweight WebAssembly `.hmod`.

**v0.11.0 — articulated multi-part Winchester 1873 (`body`, `lever`, `hammer`, `trigger`, `gate`, `cartridge`, + 3D LOD meshes), `.44-40` terminal ballistics (`14` base / `21` `1.5×` sub-tick ray-AABB headshots, smooth `> 40 m` falloff, foliage perforation, shield impact & TNT detonation), 3D spatial acoustic system (`23` `48 kHz` CC0 `.ogg` assets: supersonic cracks, Doppler whizzes, cave/valley echoes, indoor blast concussion & multiplayer 3D foley), barrel heat & water quenching, dual-zone candle wick vs. wax-burst physics, Sculk/Warden vibrations, Illager ammo drops, 14 advancements + structure chest loot, 33-option Mod Config, and `en_us` / `pt_br` / `pt_pt` localization.**

## How it plays

- Craft the **Winchester 1873** (iron ingots, oak planks, a stick, and a tripwire hook) and **.44-40 Cartridges** (gunpowder + copper ingot + iron nugget, 6 at a time) — discover forgotten unloaded rifles and `.44-40` stashes inside **Abandoned Mineshafts, Pillager Outposts, Woodland Mansions, Dungeons, and Desert Pyramids**, or recover `.44-40` rounds from **Illagers** felled by your Winchester (scaling with Looting).
- The tubular magazine holds **8 rounds**; a brass bar under the item icon, smart item tooltips (showing loaded rounds, live reserve, ballistics, and your current keybinds while hiding raw `F3+H` durability), and a configurable western HUD counter (`8 / 8 │ 36`, or `∞` in Creative) track loaded vs. reserve `.44-40` rounds. Vanilla **Anvil / Grindstone / Crafting / Mending** repair is guarded so item `damage` (`0..=8` loaded rounds) can never be corrupted or duplicated, while custom Anvil renaming preserves exact loaded ammunition.
- **Hold right click** (or toggle via **Options > Mods > Just a Rifle > Config**) to shoulder and aim down sights: the vanilla HUD crosshair (`minecraft:crosshair`) disappears automatically, the cocked hammer clears the sight line, and the rear notch + front blade (`alca_entalhe_base` + `massa_lamina`) slide dead-center onto the screen via FPS-independent second-order dynamics while FOV zooms (`0.67`), mouse turn sensitivity scales proportionally, and a subtle vignette frames the view.
- **Left click** fires one `.44-40` round (`15.0` b/t = `300 m/s`, `14.0` base damage, **`1.5×` = `21.0` damage on sub-tick 3D ray-AABB headshots** with cranial crit sparks & crimson-gold hitmarker, and smooth Hermite distance falloff beyond `40 m`):
  - **3D Spatial Acoustics**: supersonic N-wave bullet cracks (`>= 2.2 b/t`) and subsonic Doppler whizzes at closest approach, sky-light & ceiling-probed cave slapback vs. rolling valley echoes with `340 m/s` speed-of-sound arrival delay, optional indoor blast concussion muffling, and distinct soft-lead body vs. metallic armor/shield impact cues.
  - **World & Combat Physics**: perforates up to 2 leaf/vine blocks (`0.82×` velocity retention), knocks back and wears raised Shields (`4..10` durability), primes dry TNT blocks (`15`-tick short fuse) and detonates TNT Minecarts, rings Bells from afar, chimes `15`-power Target bullseyes, snuffs lit Candle wicks cleanly on top grazes while bursting wax bodies on center/base hits, shatters glass/pots/dripstone, emits attributed Sculk/Warden vibrations (`minecraft:projectile_shoot` & `minecraft:projectile_land` with `#minecraft:dampens_vibrations`), and builds **barrel heat** on rapid fire (wisping smoke above the front sight and hissing steam if dipped into water).
- **R** (remappable in **Options > Controls**) runs a **continuous per-cartridge reload loop** through the King's side loading gate (`porta_B`), finishing with a full lever rack when reloading from `0 / 8`. Firing mid-reload immediately interrupts the reload when `>= 1` round is already chambered.
- **I** (remappable in **Options > Controls**) plays a 3.4-second **weapon inspection** sequence showing both sides of the receiver, walnut stock, and brass carrier.
- **F** (swap to off-hand) **slings the rifle diagonally across the player's back** in 3rd-person (`44°` barrel-up / stock-down, adapting depth automatically over chest armor), while main-hand 3rd-person (`F5` & multiplayer) transitions dynamically across **5 two-handed postures** with synchronized remote 3D Mono reload/lever foley and brass casing ejection.

## Build and test

You need Rust 1.95+, the Horizon launcher, and `cargo-horizon-mod`:

```sh
cargo install --locked --git https://github.com/horizon-rust/horizon --rev f1bb66f353cf67f2aa4665293fc75c8d91186fc7 cargo-horizon-mod
python3 tools/verify_assets.py          # verifies manifests, JSONs, 48 kHz .ogg audio, credits, lang & PNGs
cargo horizon-mod build --wasm          # fetches the pinned SDK into .horizon/, writes mod.wasm, packs target/horizon-mod/just_a_rifle.hmod
cargo horizon-mod check target/horizon-mod/just_a_rifle.hmod
cargo test --lib                        # runs pure-Rust ballistics, audio, HUD, skin, settings, exploration & VFX unit tests
cargo horizon-mod run                   # builds and starts the game with the mod on
```

Or copy the `.hmod` into the game folder's `horizon-mods/` and add `mod.just_a_rifle:true` to `horizon-mods.txt`.

The Horizon build this mod targets is pinned in `Cargo.toml` (`[package.metadata.horizon] sdk`, Nightly 2026-11-22, `f1bb66f`). `cargo horizon-mod update` moves the pin.

## Installing

This is a **wasm** mod: install the packaged `.hmod`, no build needed.

1. Download `just_a_rifle.hmod` from the [latest release](https://github.com/celso-abreu/Just_a_Rifle/releases/latest).
2. Drop it into the instance's **`horizon-mods/`** folder (from the launcher: open the instance and use **Open the instance folder**; create `horizon-mods` if missing).
3. Play — wasm mods need no native-code consent.

Note: the launcher's *Add a native mod from GitHub* dialog is for **native** mods only; it refuses wasm packages (`not a native package`). Tagged releases (`v*`, matching `version` in `horizon-mod.toml`) are built, checked, and published by `.github/workflows/release.yml`.

## CI

Every push verifies manifests, `icon.png`, `48 kHz` Mono/Stereo `.ogg` audio & `SOUND-CREDITS.md` parity, LOD meshes, advancements, settings, and localization (`python3 tools/verify_assets.py`), builds and checks the mod (`cargo horizon-mod build --wasm` + `cargo horizon-mod check`), and runs the crate's unit test suite (`cargo test --lib`) on GitHub Actions (`.github/workflows/build-mod.yml`), reusing cached tool, SDK, toolchain, and Cargo layers between runs. The checked `just_a_rifle.hmod` is downloadable from the run's artifacts.

## Layout

```text
horizon-mod.toml            # manifest: id = "just_a_rifle", version 0.11.0, homepage/issues/credits
icon.png                    # 256x256 western brass package icon rendered from the 3D rifle & .44-40 art
Cargo.toml                  # wasm code crate + pinned Horizon SDK (f1bb66f)
CHANGELOG.md                # version history (v0.5.0 -> v0.11.0)
NEXT.md                     # v1.0.0 milestone roadmap (gunsmithing & brass reloading loop)
SOUND-CREDITS.md            # CC0 / Public Domain attribution for all 23 48 kHz .ogg audio files
src/
  lib.rs                    # entry point, network payloads, shared keys, 3rd-person poses & Hooks
  constants.rs              # IDs, ballistics & exact Blockbench [0..16] texel pivots
  ballistics.rs             # sub-tick 3D ray-AABB headshots, shield wear, foliage & falloff math
  settings.rs               # Options > Mods > Config 33-setting registry & per-channel audio mixer
  skin.rs                   # Mojang Base64 + UTF-16LE SHA-1 decoder & 18-skin fallback
  arms.rs                   # 1st-person slim (3x12x4) / wide (4x12x4) arm & sleeve meshes
  pose.rs                   # 2nd-order springs, TAC state machine, hands & sight kinematics
  client_audio.rs           # tracked foley cancellation, indoor blast concussion & sync handlers
  hud.rs                    # western brass ammo HUD, headshot hitmarker, ADS vignette & tooltips
  world_fx.rs               # reactive targets, candle/TNT/leaf FX, decals & 3D brass casings
  exploration.rs            # structure chest loot, Illager .44-40 drops & 14 advancements
  server.rs                 # authoritative firing, acoustics, barrel heat, water & reload loop
data/just_a_rifle/
  advancement/              # 14 western/marksman advancements + recipe unlocks
  horizon/item/             # winchester_1873, cartridge_44, bullet (item declarations)
  horizon/entity_type/      # bullet: thrown projectile (snowball template, 14.0 base damage)
  horizon/creative_tab/     # the mod's creative tab ("Just a Rifle")
  recipe/                   # rifle + cartridge recipes
assets/just_a_rifle/
  items/                    # 26.3 item definitions (routes GUI/ground/3rd-person to fast 3D LOD)
  models/item/              # full 336-element first-person mesh + 40-element 3D LOD item mesh
  models/special/           # 9 standalone sub-models: body, body_lod, lever, hammer, trigger,
                            #   gate, cartridge, spent casing, and terrain bullet-hole decal
  textures/item/            # 32x32 cartridge & bullet, 16x16 cutout bullet hole + 1024x1024 rifle sheet
  sounds/ + sounds.json     # 20 sound events across 23 48 kHz CC0 .ogg files + localized subtitles
  lang/                     # en_us, pt_br, pt_pt (141 keys: items, tooltips, 33 settings, 14 advancements)
models/winchester_1873.bbmodel  # Blockbench source of the rifle's 3D model
tools/
  verify_assets.py          # zero-dependency manifest, JSON, 48 kHz Vorbis, credits, lang & PNG verifier
  make_textures.py          # deterministic pure-Python generator for item, decal & icon.png art
  bbmodel_convert.py        # converts the .bbmodel into item/special JSONs and rifle_texture.png
```

## Roadmap to `v1.0.0`

See [NEXT.md](NEXT.md) — realistic gunsmithing crafting (`Octagonal Barrel` + `Receiver & Lever` + `Walnut Stock`) and optional spent `.44-40` brass casing recovery & reloading economy.
