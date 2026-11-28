# Next Steps — Just a Rifle (`v1.0.0` Milestone Roadmap)

> **Core Premise:** Keep **Just a Rifle** strictly faithful to its identity — **just one iconic firearm** (the lever-action Winchester 1873 in `.44-40`), crafted with maximum tactile, visual, acoustic, and survival depth, zero gunpack bloat, and lightweight WASM performance.

---

## Shipped in `v0.11.0`

- [x] **3D Spatial Acoustic System (`48 kHz`) & Multiplayer Foley** (`23` CC0 `.ogg` assets: supersonic `bullet_crack` + subsonic Doppler `bullet_whiz` flybys, cave vs. open-valley environmental echoes with `340 m/s` propagation delay, indoor blast concussion, flesh vs. armor/shield impact cues, and remote 3D Mono reload/lever/casing audio).
- [x] **World & Combat Ballistic Reactivity** (up to 2-block leaf/foliage perforation, short-fuse TNT priming + TNT Minecart detonation & `Powder Keg` 14th advancement, dual-zone Candle wick-snuff vs. wax-burst physics, heavy Shield knockback & wear, Sculk/Warden `GameEvent` vibrations, Illager `.44-40` combat drops, and rapid-fire barrel heat + water steam quenching).
- [x] **Magazine State Integrity & Codebase Modularization** (Anvil/Grindstone/Crafting/Mending ammo-damage guards, `src/client_audio.rs` + `src/hud.rs` extraction, full WASM world-exit state resets & disconnected-player map pruning, `pt_pt.json` European Portuguese localization, complete `SOUND-CREDITS.md` coverage, and hardened `tools/verify_assets.py` checks).

## Shipped in `v0.10.0`

- [x] **3D GUI / World LOD & Zero-Lag Inventory Rendering** (`winchester_1873_lod` + `winchester_body_lod`, lossless internal face culling, and `20 Hz` WASM caches).
- [x] **HUD `.44-40` Reserve Counter & Smart Dynamic Tooltips** (`8 / 8 │ [mini .44-40] 36` with `∞` in Creative, localized tooltips with live remapped keybinds `[R]`, `[I]`, `[F]`, and suppressed `F3+H` durability).
- [x] **`.44-40` Headshots & Long-Range Ballistics** (`14.0` base damage, `1.5×` = `21.0` sub-tick 3D ray-AABB headshots, smooth `> 40 m` Hermite falloff, crimson-gold headshot hitmarker, and `Between the Eyes`).
- [x] **Dynamic Two-Handed 3rd-Person Arm Poses (`F5` & Multiplayer)** (`hip_ready`, `sprint_carry`, `CrossbowHold` ADS, `lever_cycle`, `reload`, and `Empty` off-hand back sling).
- [x] **Mod Identity & Categorized Config** (`256×256` root `icon.png`, `homepage`/`issues`/`credits` manifest metadata, and `[Audio]` / `[Feedback]` / `[Controls]` / `[Camera]` / `[HUD]` / `[World]` Config labels).

---

## Remaining Milestone for `v1.0.0` (Full Survival Release)

### 1. Gunsmithing & `.44-40` Brass Reloading Economy
- **Goal:** Give the Winchester 1873 authentic mid-game frontier progression and connect ejected 3D brass shells directly to Survival gameplay.
- **Planned Scope:**
  1. **Three-Part Gunsmithing Assembly:** Craft `Octagonal Rifled Barrel` (iron + copper), `Lever Receiver Mechanism` (iron + tripwire hook + redstone/nuggets), and `Oiled Walnut Stock` (dark oak / oak + honeycomb/wax) to assemble the Winchester 1873 at a crafting table.
  2. **Spent `.44-40` Brass Casing Recovery (Optional Toggle):** Walking over spent `.44-40` brass casings resting on the ground (or enabling physical brass drops) recovers `Empty .44-40 Casing`, allowing players to hand-load `.44-40 Cartridges` at **half the copper cost** (`Empty Casing + Gunpowder + Iron Nugget`).
  3. **Gunsmith Advancements & Final `v1.0.0` Balance Pass:** Advancement for hand-loading recovered `.44-40` brass and final Survival loot/recipe tuning.
