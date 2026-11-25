# Next Steps — Just a Rifle (`v1.0.0` Milestone Roadmap)

> **Core Premise:** Keep **Just a Rifle** strictly faithful to its identity — **just one iconic firearm** (the lever-action Winchester 1873 in `.44-40`), crafted with maximum tactile, visual, acoustic, and survival depth, zero gunpack bloat, and lightweight WASM performance.

---

## Shipped in `v0.10.0`

- [x] **3D GUI / World LOD & Zero-Lag Inventory Rendering** (`winchester_1873_lod` + `winchester_body_lod`, lossless internal face culling, and `20 Hz` WASM caches).
- [x] **HUD `.44-40` Reserve Counter & Smart Dynamic Tooltips** (`8 / 8 │ [mini .44-40] 36` with `∞` in Creative, localized `en_us` / `pt_br` tooltips with live remapped keybinds `[R]`, `[I]`, `[F]`, and suppressed `F3+H` durability).
- [x] **`.44-40` Headshots & Long-Range Ballistics** (`14.0` base damage, `1.5×` = `21.0` sub-tick 3D ray-AABB headshots, smooth `> 40 m` Hermite falloff, crimson-gold headshot hitmarker, and the 13th advancement `Between the Eyes`).
- [x] **Dynamic Two-Handed 3rd-Person Arm Poses (`F5` & Multiplayer)** (`hip_ready`, `sprint_carry`, `CrossbowHold` ADS, `lever_cycle`, `reload`, and `Empty` off-hand back sling).
- [x] **Mod Identity & 31-Option Categorized Config** (`256×256` root `icon.png`, `homepage`/`issues`/`credits` manifest metadata, and `[Audio]` / `[Feedback]` / `[Controls]` / `[Camera]` / `[HUD]` / `[World]` Config labels).

---

## Remaining Milestone for `v1.0.0` (Full Survival Release)

### 1. Gunsmithing & `.44-40` Brass Reloading Economy
- **Goal:** Give the Winchester 1873 authentic mid-game frontier progression and connect ejected 3D brass shells directly to Survival gameplay.
- **Planned Scope:**
  1. **Three-Part Gunsmithing Assembly:** Craft `Octagonal Rifled Barrel` (iron + copper), `Lever Receiver Mechanism` (iron + tripwire hook + redstone/nuggets), and `Oiled Walnut Stock` (dark oak / oak + honeycomb/wax) to assemble the Winchester 1873 at a crafting table.
  2. **Spent `.44-40` Brass Casing Recovery (Optional Toggle):** Walking over spent `.44-40` brass casings resting on the ground (or enabling physical brass drops) recovers `Empty .44-40 Casing`, allowing players to hand-load `.44-40 Cartridges` at **half the copper cost** (`Empty Casing + Gunpowder + Iron Nugget`).
  3. **Gunsmith Advancements & Final `v1.0.0` Balance Pass:** Advancement for hand-loading recovered `.44-40` brass and final Survival loot/recipe tuning.
