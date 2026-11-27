//! Reactive world targets, per-material bullet impact audio & VFX, terrain decals, and
//! 3D bouncing `.44-40` brass shell casings.

use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};

use mod_api::{
    BlockUpdateFlags, CameraType, ClientHost, Compound, GameEventContext, GameRuleValue, Host,
    HostBlockStates, IPayloadContext, LevelRef, LightCoords, LightLayer, ModelInstance, Overlay,
    ParticleOptions, ProjectileImpactEvent, ServerHost, SoundSource, Tag, VarInt,
    WorldDrawContext,
};

use crate::constants::*;
use crate::server::{
    consume_bullet_water_splash_on_impact, flush_bullet_flyby_on_impact,
    try_perforate_foliage_on_impact,
};
use crate::{
    holding_rifle_recently, is_point_in_water, play_local_positional_foley, BulletImpactSync,
    BELL_BLOCK_KEY, BLOCK_PARTICLE_KEY, BUBBLE_KEY, BUBBLE_POP_KEY, BULLET_IMPACT_SYNC,
    CHORUS_FLOWER_BLOCK_KEY, CRIT_KEY, DECORATED_POT_BLOCK_KEY, FLAME_KEY, LAST_EJECTOR_VIEW_X,
    LAST_EJECTOR_VIEW_Y, LAST_EJECTOR_VIEW_Z, LAST_WORLD_FOV_BITS, POINTED_DRIPSTONE_BLOCK_KEY,
    POOF_KEY, RIFLE_MODELS, SMOKE_KEY, TARGET_BLOCK_KEY, WHITE_SMOKE_KEY,
};

static WORLD_EFFECTS: OnceLock<Mutex<WorldEffectsState>> = OnceLock::new();

#[derive(Clone)]
struct BulletHoleDecal {
    dimension: String,
    block: [i32; 3],
    face: [i32; 3],
    at: [f64; 3],
    roll_quarter: u8,
    slot_bias: u8,
    age: f32,
    light: LightCoords,
}

#[derive(Clone)]
struct SpentCasing {
    dimension: String,
    pos: [f64; 3],
    vel: [f64; 3],
    yaw_deg: f32,
    pitch_deg: f32,
    roll_deg: f32,
    yaw_vel: f32,
    pitch_vel: f32,
    roll_vel: f32,
    bounces: u8,
    settled: bool,
    age: f32,
    light: LightCoords,
}

struct WorldEffectsState {
    active_dimension: String,
    holes: Vec<BulletHoleDecal>,
    casings: Vec<SpentCasing>,
    next_slot_bias: u8,
    prune_timer: f32,
    rng: u32,
}

impl WorldEffectsState {
    fn new() -> Self {
        Self {
            active_dimension: DEFAULT_DIMENSION.into(),
            holes: Vec::with_capacity(MAX_BULLET_HOLES),
            casings: Vec::with_capacity(MAX_SPENT_CASINGS),
            next_slot_bias: 0,
            prune_timer: 0.0,
            rng: 0xA341_316C,
        }
    }

    fn next_unit_f32(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        ((self.rng >> 8) as f32) * (1.0 / 16_777_216.0)
    }
}

fn world_effects() -> &'static Mutex<WorldEffectsState> {
    WORLD_EFFECTS.get_or_init(|| Mutex::new(WorldEffectsState::new()))
}

fn normalize_cardinal_face(face: [i32; 3]) -> [i32; 3] {
    let ax = face[0].abs();
    let ay = face[1].abs();
    let az = face[2].abs();
    if ax >= ay && ax >= az && ax > 0 {
        [face[0].signum(), 0, 0]
    } else if ay >= ax && ay >= az && ay > 0 {
        [0, face[1].signum(), 0]
    } else if az > 0 {
        [0, 0, face[2].signum()]
    } else {
        [0, 1, 0]
    }
}

fn sample_world_light(host: &dyn ClientHost, dimension: &str, pos: [i32; 3]) -> LightCoords {
    let default_light = LightCoords::new(0, 15).unwrap_or(LightCoords::FULL_BRIGHT);
    let Some(level) = host.level(dimension) else {
        return default_light;
    };
    let block = level
        .brightness(host, LightLayer::Block, pos)
        .unwrap_or(0)
        .clamp(0, 15) as u8;
    let sky = level
        .brightness(host, LightLayer::Sky, pos)
        .unwrap_or(15)
        .clamp(0, 15) as u8;
    LightCoords::new(block, sky).unwrap_or(default_light)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImpactMaterial {
    Metal,
    Crystal,
    Glass,
    Wood,
    Earth,
    Soft,
    Stone,
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

/// Extracts `"minecraft:bell"` from `"minecraft:bell[attachment=ceiling,facing=north]"`.
fn parse_block_state_id(state_str: &str) -> &str {
    state_str
        .split_once('[')
        .map_or(state_str, |(id, _)| id)
        .trim()
}

/// Reads a single property from a serialized block-state string (`"key=val"` inside `[...]`).
fn parse_state_property<'a>(state_str: &'a str, key: &str) -> Option<&'a str> {
    let (_, rest) = state_str.split_once('[')?;
    let props = rest.strip_suffix(']').unwrap_or(rest);
    for entry in props.split(',') {
        let (k, v) = entry.split_once('=')?;
        if k.trim() == key {
            return Some(v.trim());
        }
    }
    None
}

/// Returns an updated serialized block-state string with `key` set to `new_val`,
/// or `None` if `key` is absent or already equal to `new_val`.
fn replace_state_property(state_str: &str, key: &str, new_val: &str) -> Option<String> {
    let (id, rest) = state_str.split_once('[')?;
    let props = rest.strip_suffix(']')?;
    let mut changed = false;
    let mut out = String::with_capacity(state_str.len() + 4);
    out.push_str(id);
    out.push('[');
    for (idx, entry) in props.split(',').enumerate() {
        if idx > 0 {
            out.push(',');
        }
        let (k, v) = entry.split_once('=')?;
        let k_trim = k.trim();
        let v_trim = v.trim();
        out.push_str(k_trim);
        out.push('=');
        if k_trim == key {
            if v_trim == new_val {
                return None;
            }
            out.push_str(new_val);
            changed = true;
        } else {
            out.push_str(v_trim);
        }
    }
    out.push(']');
    changed.then_some(out)
}

/// Classifies any Vanilla or modded block into an acoustic/ballistic surface family using both
/// its canonical ID and its registered `BlockSoundType` (`hit` / `break_sound`).
fn classify_impact_material(block_id: &str, hit_sound: &str, break_sound: &str) -> ImpactMaterial {
    let id = block_id.strip_prefix("minecraft:").unwrap_or(block_id);

    if contains_any(id, &["amethyst"])
        || contains_any(hit_sound, &["amethyst"])
        || contains_any(break_sound, &["amethyst"])
    {
        return ImpactMaterial::Crystal;
    }

    if contains_any(
        hit_sound,
        &[
            "anvil",
            "chain",
            "copper",
            "lantern",
            "metal",
            "netherite",
            "heavy_core",
            "lodestone",
            "vault",
            "spawner",
        ],
    ) || contains_any(
        id,
        &[
            "iron_block",
            "raw_iron_block",
            "iron_bars",
            "iron_door",
            "iron_trapdoor",
            "gold_block",
            "raw_gold_block",
            "copper",
            "anvil",
            "bell",
            "chain",
            "lantern",
            "cauldron",
            "hopper",
            "netherite_block",
            "heavy_core",
            "lightning_rod",
            "weighted_pressure_plate",
        ],
    ) {
        return ImpactMaterial::Metal;
    }

    if contains_any(hit_sound, &["glass"])
        || contains_any(break_sound, &["glass"])
        || contains_any(
            id,
            &[
                "glass",
                "sea_lantern",
                "glowstone",
                "redstone_lamp",
                "beacon",
                "ice",
            ],
        )
    {
        return ImpactMaterial::Glass;
    }

    if contains_any(
        hit_sound,
        &["wood", "bamboo", "stem", "ladder", "scaffolding", "chiseled_bookshelf"],
    ) || contains_any(
        id,
        &[
            "planks",
            "log",
            "wood",
            "stem",
            "hyphae",
            "bamboo_block",
            "bamboo_mosaic",
            "barrel",
            "chest",
            "bookshelf",
            "lectern",
            "crafting_table",
            "cartography_table",
            "fletching_table",
            "loom",
            "note_block",
            "jukebox",
            "beehive",
            "composter",
        ],
    ) {
        return ImpactMaterial::Wood;
    }

    if id == "target"
        || id == "hay_block"
        || contains_any(
            hit_sound,
            &[
                "wool",
                "snow",
                "moss",
                "azalea",
                "cherry_leaves",
                "vine",
                "crop",
                "fungus",
                "slime",
                "honey",
                "sponge",
                "sculk",
                "wart_block",
                "shroomlight",
                "cobweb",
                "candle",
            ],
        )
        || contains_any(
            id,
            &[
                "wool",
                "carpet",
                "_bed",
                "leaves",
                "sponge",
                "snow",
                "moss",
                "slime_block",
                "honey_block",
                "honeycomb_block",
                "cactus",
                "melon",
                "pumpkin",
            ],
        )
    {
        return ImpactMaterial::Soft;
    }

    if contains_any(
        hit_sound,
        &[
            "gravel",
            "grass",
            "sand",
            "mud",
            "soul_sand",
            "soul_soil",
            "rooted_dirt",
            "wet_grass",
        ],
    ) || matches!(
        id,
        "dirt"
            | "coarse_dirt"
            | "rooted_dirt"
            | "grass_block"
            | "podzol"
            | "mycelium"
            | "dirt_path"
            | "farmland"
            | "clay"
            | "sand"
            | "red_sand"
            | "gravel"
            | "mud"
            | "packed_mud"
            | "soul_sand"
            | "soul_soil"
    ) {
        return ImpactMaterial::Earth;
    }

    ImpactMaterial::Stone
}

fn is_heavy_metal_surface(block_id: &str, hit_sound: &str) -> bool {
    let id = block_id.strip_prefix("minecraft:").unwrap_or(block_id);
    contains_any(hit_sound, &["anvil", "netherite", "heavy_core", "lodestone"])
        || contains_any(
            id,
            &[
                "anvil",
                "iron_block",
                "raw_iron_block",
                "iron_door",
                "iron_trapdoor",
                "netherite",
                "heavy_core",
                "cauldron",
                "hopper",
                "lodestone",
            ],
        )
}

/// Thin/delicate Vanilla blocks that a `300 m/s` `.44-40` lead slug shatters directly in
/// `handle_bullet_terrain_impact` (alongside `decorated_pot` and `chorus_flower`, which Horizon's
/// `projectile_breaks_block` shatters natively right after our hook).
fn is_fragile_bullet_shatter_block(block_id: &str, in_flower_pots_tag: bool) -> bool {
    if in_flower_pots_tag {
        return true;
    }
    let id = block_id.strip_prefix("minecraft:").unwrap_or(block_id);
    id == "glass_pane"
        || id.ends_with("_stained_glass_pane")
        || id == "pointed_dripstone"
        || id == "sulfur_spike"
        || id == "flower_pot"
        || id.starts_with("potted_")
}

/// Exact match to Horizon `redstone.rs::bell_hit` (`click_y <= 0.8124` + horizontal attachment
/// axis check): returns `true` when Horizon's native `bell_projectile_hit` will ALREADY ring
/// the bell after `projectile_impact`, so we only trigger `block_event` on off-angle/vertical shots
/// (such as shooting up into a village bell tower from below) and never double-ring.
fn vanilla_bell_will_ring(state_str: &str, face: [i32; 3], local_hit_y: f64) -> bool {
    let f = normalize_cardinal_face(face);
    if f[1] != 0 || local_hit_y > f64::from(0.8124_f32) {
        return false;
    }
    let face_x_axis = f[0] != 0;
    let facing_x_axis = matches!(
        parse_state_property(state_str, "facing"),
        Some("east" | "west")
    );
    match parse_state_property(state_str, "attachment") {
        Some("floor") => facing_x_axis == face_x_axis,
        Some("single_wall" | "double_wall") => facing_x_axis != face_x_axis,
        Some("ceiling") => true,
        _ => false,
    }
}

/// Chooses a horizontal 3D `Direction` (`2=North, 3=South, 4=West, 5=East`) so `BellRenderer`
/// visibly swings the bronze bell body even when struck from straight below or above.
fn bell_swing_direction_3d(state_str: &str, face: [i32; 3]) -> i32 {
    let f = normalize_cardinal_face(face);
    let facing_dir = match parse_state_property(state_str, "facing") {
        Some("south") => 3,
        Some("west") => 4,
        Some("east") => 5,
        _ => 2,
    };
    let attachment = parse_state_property(state_str, "attachment");
    if matches!(attachment, Some("single_wall" | "double_wall")) {
        // Wall-mounted bells swing perpendicular to their wall bracket axis.
        return match facing_dir {
            2 | 3 => {
                if f[0] < 0 {
                    4
                } else {
                    5
                }
            }
            _ => {
                if f[2] < 0 {
                    2
                } else {
                    3
                }
            }
        };
    }
    if matches!(attachment, Some("floor")) {
        // Floor-mounted bells swing along their `facing` axis.
        return facing_dir;
    }
    match f {
        [0, 0, -1] => 2,
        [0, 0, 1] => 3,
        [-1, 0, 0] => 4,
        [1, 0, 0] => 5,
        _ => facing_dir,
    }
}

/// Computes Vanilla `TargetBlock` bullseye signal strength (`1..=15`) from the exact fractional
/// hit coordinates on the struck block face.
fn compute_target_bullseye_power(block: [i32; 3], face: [i32; 3], at: [f64; 3]) -> i32 {
    let f = normalize_cardinal_face(face);
    let fx = (at[0] - f64::from(block[0])).clamp(0.0, 1.0);
    let fy = (at[1] - f64::from(block[1])).clamp(0.0, 1.0);
    let fz = (at[2] - f64::from(block[2])).clamp(0.0, 1.0);
    let dist = if f[1] != 0 {
        (fx - 0.5).abs().max((fz - 0.5).abs())
    } else if f[0] != 0 {
        (fy - 0.5).abs().max((fz - 0.5).abs())
    } else {
        (fx - 0.5).abs().max((fy - 0.5).abs())
    };
    (((1.0 - dist * 2.0).clamp(0.0, 1.0) * 15.0).ceil() as i32).clamp(1, 15)
}

/// Verifies that at least one collision/outline box at `block` presents a flat surface on `face`
/// near `local_hit` wide enough (`>= min_span` on both tangent axes) to hold a bullet-hole decal
/// without overhanging into empty air around thin fences, chains, bars, candles, or bells.
fn face_supports_bullet_decal(
    boxes: &[[f64; 6]],
    local_hit: [f64; 3],
    face: [i32; 3],
    min_span: f64,
) -> bool {
    if boxes.is_empty() {
        return false;
    }
    let f = normalize_cardinal_face(face);
    let plane_eps = 0.085;
    let uv_pad = 0.06;
    boxes.iter().any(|b| match f {
        [1, 0, 0] => {
            (local_hit[0] - b[3]).abs() <= plane_eps
                && (b[4] - b[1]) >= min_span
                && (b[5] - b[2]) >= min_span
                && local_hit[1] >= b[1] - uv_pad
                && local_hit[1] <= b[4] + uv_pad
                && local_hit[2] >= b[2] - uv_pad
                && local_hit[2] <= b[5] + uv_pad
        }
        [-1, 0, 0] => {
            (local_hit[0] - b[0]).abs() <= plane_eps
                && (b[4] - b[1]) >= min_span
                && (b[5] - b[2]) >= min_span
                && local_hit[1] >= b[1] - uv_pad
                && local_hit[1] <= b[4] + uv_pad
                && local_hit[2] >= b[2] - uv_pad
                && local_hit[2] <= b[5] + uv_pad
        }
        [0, 1, 0] => {
            (local_hit[1] - b[4]).abs() <= plane_eps
                && (b[3] - b[0]) >= min_span
                && (b[5] - b[2]) >= min_span
                && local_hit[0] >= b[0] - uv_pad
                && local_hit[0] <= b[3] + uv_pad
                && local_hit[2] >= b[2] - uv_pad
                && local_hit[2] <= b[5] + uv_pad
        }
        [0, -1, 0] => {
            (local_hit[1] - b[1]).abs() <= plane_eps
                && (b[3] - b[0]) >= min_span
                && (b[5] - b[2]) >= min_span
                && local_hit[0] >= b[0] - uv_pad
                && local_hit[0] <= b[3] + uv_pad
                && local_hit[2] >= b[2] - uv_pad
                && local_hit[2] <= b[5] + uv_pad
        }
        [0, 0, 1] => {
            (local_hit[2] - b[5]).abs() <= plane_eps
                && (b[3] - b[0]) >= min_span
                && (b[4] - b[1]) >= min_span
                && local_hit[0] >= b[0] - uv_pad
                && local_hit[0] <= b[3] + uv_pad
                && local_hit[1] >= b[1] - uv_pad
                && local_hit[1] <= b[4] + uv_pad
        }
        _ => {
            (local_hit[2] - b[2]).abs() <= plane_eps
                && (b[3] - b[0]) >= min_span
                && (b[4] - b[1]) >= min_span
                && local_hit[0] >= b[0] - uv_pad
                && local_hit[0] <= b[3] + uv_pad
                && local_hit[1] >= b[1] - uv_pad
                && local_hit[1] <= b[4] + uv_pad
        }
    })
}

/// Deterministic `[-1.0, 1.0]` jitter from impact coordinates so rapid lever-action hits vary
/// subtly in pitch without locking any shared state on the server.
fn impact_pitch_jitter(block: [i32; 3], at: [f64; 3]) -> f32 {
    let qx = (at[0] * 1024.0) as i64 as u64;
    let qy = (at[1] * 1024.0) as i64 as u64;
    let qz = (at[2] * 1024.0) as i64 as u64;
    let mut h = (block[0] as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add((block[1] as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9))
        .wrapping_add((block[2] as u64).wrapping_mul(0x94D0_49BB_1331_11EB))
        ^ qx.rotate_left(13)
        ^ qy.rotate_left(29)
        ^ qz.rotate_left(47);
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    (((h & 0xFFFF) as f32) * (1.0 / 32_767.5) - 1.0).clamp(-1.0, 1.0)
}

/// Checks Vanilla/Horizon 26.3 `projectiles_can_break_blocks`, `block_drops`, and the shooter's
/// `Player.mayBuild` (so Adventure/Spectator players cannot shatter blocks with gunfire).
fn bullet_break_permissions(
    host: &dyn Host,
    level: LevelRef,
    owner_entity_id: u32,
) -> (bool, bool) {
    if matches!(
        level.game_rule(host, RULE_PROJECTILES_BREAK_BLOCKS),
        Ok(Some(GameRuleValue::Bool(false)))
    ) {
        return (false, false);
    }
    if owner_entity_id != 0
        && let Some(players) = level.online_players(host)
        && let Some(shooter) = players
            .into_iter()
            .find(|p| p.entity().id() == owner_entity_id)
        && shooter.may_build(host) == Some(false)
    {
        return (false, false);
    }
    let drop_items = !matches!(
        level.game_rule(host, RULE_BLOCK_DROPS),
        Ok(Some(GameRuleValue::Bool(false)))
    );
    (true, drop_items)
}

pub(crate) fn handle_bullet_terrain_impact(
    host: &mut dyn ServerHost,
    event: &ProjectileImpactEvent<'_>,
) {
    let face = normalize_cardinal_face(event.face);
    let impact_fx_pos = [
        event.at[0] + f64::from(face[0]) * 0.055,
        event.at[1] + f64::from(face[1]) * 0.055,
        event.at[2] + f64::from(face[2]) * 0.055,
    ];
    let local_hit = [
        event.at[0] - f64::from(event.block[0]),
        event.at[1] - f64::from(event.block[1]),
        event.at[2] - f64::from(event.block[2]),
    ];

    let Some(level) = host.level(event.dimension) else {
        return;
    };
    consume_bullet_water_splash_on_impact(host, level, event.at);
    flush_bullet_flyby_on_impact(host, level, event.owner, event.at, None);

    let underwater =
        is_point_in_water(host, level, impact_fx_pos) || is_point_in_water(host, level, event.at);

    let Some(hit_state) = level.block_state(host, event.block) else {
        return;
    };
    let hit_block = hit_state.block();
    let state_str = host.block_state_string(hit_state).unwrap_or_default();
    let block_id = parse_block_state_id(&state_str);
    let sound_info = host.block_states().sound(hit_state);
    let hit_sound = sound_info
        .as_ref()
        .map_or(SND_IMPACT_HIT, |s| s.hit.as_ref());
    let break_sound = sound_info
        .as_ref()
        .map_or(SND_IMPACT_HIT, |s| s.break_sound.as_ref());
    let place_sound = sound_info
        .as_ref()
        .map_or(SND_IMPACT_HIT, |s| s.place.as_ref());
    let fall_sound = sound_info
        .as_ref()
        .map_or(SND_IMPACT_HIT, |s| s.fall.as_ref());
    let base_pitch = sound_info.as_ref().map_or(1.0, |s| s.pitch).clamp(0.5, 1.8);

    let material = classify_impact_material(block_id, hit_sound, break_sound);
    let jitter = impact_pitch_jitter(event.block, event.at);

    let is_bell = BELL_BLOCK_KEY.get().copied() == Some(hit_block) || block_id == BELL_BLOCK;
    let is_target =
        TARGET_BLOCK_KEY.get().copied() == Some(hit_block) || block_id == TARGET_BLOCK;
    let is_decorated_pot = DECORATED_POT_BLOCK_KEY.get().copied() == Some(hit_block)
        || block_id == DECORATED_POT_BLOCK;
    let is_chorus_flower = CHORUS_FLOWER_BLOCK_KEY.get().copied() == Some(hit_block)
        || block_id == CHORUS_FLOWER_BLOCK;
    let is_dripstone = POINTED_DRIPSTONE_BLOCK_KEY.get().copied() == Some(hit_block)
        || block_id == POINTED_DRIPSTONE_BLOCK;
    let is_tnt = block_id == TNT_BLOCK;
    let in_flower_pots = host.block_in_tag(hit_block, TAG_FLOWER_POTS).unwrap_or(false);
    let in_candles = host.block_in_tag(hit_block, TAG_CANDLES).unwrap_or(false);
    let in_leaves = host.block_in_tag(hit_block, TAG_LEAVES).unwrap_or(false);

    let (can_break, drop_items) = bullet_break_permissions(host, level, event.owner);
    let shooter_dist =
        crate::exploration::resolve_shooter_and_distance(host, level, event.owner, event.at);
    let source_ent = if event.owner != 0 {
        host.entity(event.owner)
    } else {
        shooter_dist.as_ref().map(|(s, _)| s.entity())
    };

    // Emit Sculk/Warden `minecraft:projectile_land` downrange vibration (muffled automatically
    // when `affected_state` belongs to `#minecraft:dampens_vibrations`).
    let _ = level.game_event(
        host,
        GAME_EVENT_PROJECTILE_LAND,
        event.at,
        GameEventContext {
            source_entity: source_ent,
            affected_state: Some(hit_state),
        },
    );

    // Horizon natively shatters `decorated_pot` and `chorus_flower` right after `projectile_impact`
    // when `can_break` holds; we shatter thin window panes, flower pots, and stalactites directly.
    let native_will_shatter = can_break && (is_decorated_pot || is_chorus_flower);
    let mut shattered_now = false;
    let mut suppress_decal = native_will_shatter
        || is_bell
        || is_dripstone
        || is_tnt
        || in_flower_pots
        || in_candles
        || in_leaves;

    if !suppress_decal
        && let Some(outline) = host.state_outline(hit_state)
        && !face_supports_bullet_decal(&outline, local_hit, face, BULLET_HOLE_MIN_FACE_SPAN)
    {
        suppress_decal = true;
    }

    // 1. Reactive Vanilla Blocks: Bell, Target Bullseye, Fragile Shatter, and Lit Candles.
    if is_bell {
        if !vanilla_bell_will_ring(&state_str, face, local_hit[1]) {
            let bell_key = BELL_BLOCK_KEY.get().copied().unwrap_or(hit_block);
            let swing_dir = bell_swing_direction_3d(&state_str, face);
            let bell_center = [
                f64::from(event.block[0]) + 0.5,
                f64::from(event.block[1]) + 0.5,
                f64::from(event.block[2]) + 0.5,
            ];
            level.block_event(host, event.block, bell_key, 1, swing_dir);
            level.play_sound(
                host,
                SND_BELL_USE,
                bell_center,
                SoundSource::Blocks,
                2.0,
                1.0 + 0.03 * jitter,
            );
            level.game_event(
                host,
                GAME_EVENT_BLOCK_CHANGE,
                event.at,
                GameEventContext::default(),
            );
            if let Some((shooter, dist)) = shooter_dist
                && dist >= ADV_BELL_MIN_DIST
            {
                crate::exploration::grant_mod_criterion(host, shooter, TRIG_BELL_RINGER);
            }
        }
    } else if is_target {
        // Leave `minecraft:target` block state untouched so Horizon's native `target_block_hit`
        // drives exact 8-tick Redstone power, stats, and Bullseye advancements, while we add
        // crisp acoustic range feedback on accurate hits (`power >= 12`).
        let power = compute_target_bullseye_power(event.block, face, event.at);
        if power == 15 {
            level.play_sound(
                host,
                SND_TARGET_BULLSEYE,
                event.at,
                SoundSource::Players,
                0.95,
                1.56,
            );
            level.play_sound(
                host,
                SND_TARGET_BULLSEYE,
                event.at,
                SoundSource::Players,
                0.65,
                1.98,
            );
            if let Some((shooter, dist)) = shooter_dist
                && dist >= ADV_DEADEYE_MIN_DIST
            {
                crate::exploration::grant_mod_criterion(host, shooter, TRIG_DEADEYE);
            }
        } else if power >= 12 {
            level.play_sound(
                host,
                SND_TARGET_BULLSEYE,
                event.at,
                SoundSource::Players,
                0.68,
                1.24,
            );
        }
    } else if in_candles
        && !underwater
        && parse_state_property(&state_str, "lit") == Some("true")
        && let Some(unlit_str) = replace_state_property(&state_str, "lit", "false")
        && let Some(unlit_state) = host.block_state_id(&unlit_str)
        && matches!(
            level.set_block(
                host,
                event.block,
                unlit_state,
                BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS,
            ),
            Ok(true)
        )
    {
        // Trick-shot: snuff out a lit candle flame without breaking the candle.
        level.play_sound(
            host,
            SND_CANDLE_EXTINGUISH,
            event.at,
            SoundSource::Players,
            0.85,
            1.05 + 0.06 * jitter,
        );
        if let Some((shooter, _)) = shooter_dist {
            crate::exploration::grant_mod_criterion(host, shooter, TRIG_CANDLE_SNUFFER);
        }
        if let Some(smoke) = SMOKE_KEY.get().copied() {
            let smoke_opts = ParticleOptions::simple(smoke);
            level.send_particles(
                host,
                &smoke_opts,
                None,
                [
                    f64::from(event.block[0]) + 0.5,
                    f64::from(event.block[1]) + 0.55,
                    f64::from(event.block[2]) + 0.5,
                ],
                [0.04, 0.06, 0.04],
                0.015,
                5,
            );
        }
    } else if in_leaves && !underwater {
        // Soft-lead `.44-40` shears cleanly through up to 2 dry canopy leaf blocks (`72%` speed
        // retention per leaf) while bursting sheared leaf particles on entry.
        let _ = try_perforate_foliage_on_impact(host, level, event.owner, event.block, event.at);
    } else if is_tnt
        && !underwater
        && can_break
        && !matches!(
            level.game_rule(host, RULE_TNT_EXPLODES),
            Ok(Some(GameRuleValue::Bool(false)))
        )
        && let Some(air_state) = host.block_state_id(AIR_BLOCK)
        && matches!(
            level.set_block(
                host,
                event.block,
                air_state,
                BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS,
            ),
            Ok(true)
        )
    {
        // Hot `.44-40` blackpowder spark ignites TNT blocks on a rapid `0.9 s` (`18`-tick) fuse!
        shattered_now = true;
        suppress_decal = true;
        let tnt_pos = [
            f64::from(event.block[0]) + 0.5,
            f64::from(event.block[1]),
            f64::from(event.block[2]) + 0.5,
        ];
        let mut nbt = Compound::new();
        nbt.insert("fuse".into(), Tag::Short(TNT_BULLET_FUSE_TICKS));
        if let Some(tnt_eid) = host.summon(event.dimension, ENTITY_TNT, tnt_pos, nbt)
            && let Some(tnt_ent) = host.entity(tnt_eid)
        {
            let ang = f64::from(jitter) * std::f64::consts::PI;
            let _ = tnt_ent.set_velocity(host, [-ang.sin() * 0.02, 0.20, -ang.cos() * 0.02]);
        }
        level.play_sound(
            host,
            SND_TNT_PRIMED,
            tnt_pos,
            SoundSource::Blocks,
            1.0,
            1.08 + 0.04 * jitter,
        );
        let _ = level.game_event(
            host,
            GAME_EVENT_PRIME_FUSE,
            tnt_pos,
            GameEventContext {
                source_entity: source_ent,
                affected_state: None,
            },
        );
        if let Some(flame) = FLAME_KEY.get().copied() {
            let flame_opts = ParticleOptions::simple(flame);
            level.send_particles(
                host,
                &flame_opts,
                None,
                impact_fx_pos,
                [0.06, 0.06, 0.06],
                0.04,
                6,
            );
        }
        if let Some(smoke) = SMOKE_KEY.get().copied() {
            let smoke_opts = ParticleOptions::simple(smoke);
            level.send_particles(
                host,
                &smoke_opts,
                None,
                impact_fx_pos,
                [0.08, 0.08, 0.08],
                0.03,
                8,
            );
        }
        if let Some((shooter, _)) = shooter_dist {
            crate::exploration::grant_mod_criterion(host, shooter, TRIG_POWDER_KEG);
        }
    } else if can_break && (is_dripstone || is_fragile_bullet_shatter_block(block_id, in_flower_pots))
    {
        if let Ok(Some(_)) = level.destroy_block(host, event.block, drop_items, None, false) {
            shattered_now = true;
            suppress_decal = true;
        }
    }

    // 2. Per-Material Impact Audio & Surface VFX.
    if underwater {
        level.play_sound(
            host,
            hit_sound,
            event.at,
            SoundSource::Players,
            0.48,
            (base_pitch * 0.68 + 0.03 * jitter).clamp(0.48, 1.15),
        );
        level.play_sound(
            host,
            SND_WATER_BUBBLES,
            event.at,
            SoundSource::Players,
            0.42,
            1.35 + 0.05 * jitter,
        );
    } else if shattered_now || native_will_shatter {
        // `destroy_block` / `projectile_breaks_block` already emits Vanilla's full shatter sound
        // & break particles; layer a crisp high-velocity bullet-snap transient on top.
        level.play_sound(
            host,
            hit_sound,
            event.at,
            SoundSource::Players,
            0.42,
            (base_pitch * 1.28 + 0.04 * jitter).clamp(0.75, 1.85),
        );
    } else {
        match material {
            ImpactMaterial::Metal => {
                let heavy = is_heavy_metal_surface(block_id, hit_sound);
                level.play_sound(
                    host,
                    place_sound,
                    event.at,
                    SoundSource::Players,
                    0.74,
                    (base_pitch * 1.12 + 0.05 * jitter).clamp(0.75, 1.75),
                );
                level.play_sound(
                    host,
                    if heavy {
                        SND_RICOCHET_METAL_HEAVY
                    } else {
                        SND_RICOCHET_METAL_SHEET
                    },
                    event.at,
                    SoundSource::Players,
                    if is_bell { 0.44 } else { 0.54 },
                    (if heavy { 1.66 } else { 1.78 }) + 0.08 * jitter,
                );
            }
            ImpactMaterial::Crystal => {
                level.play_sound(
                    host,
                    hit_sound,
                    event.at,
                    SoundSource::Players,
                    0.78,
                    (base_pitch * 1.15 + 0.05 * jitter).clamp(0.75, 1.85),
                );
                level.play_sound(
                    host,
                    SND_CRYSTAL_CHIME,
                    event.at,
                    SoundSource::Players,
                    0.72,
                    1.42 + 0.12 * jitter,
                );
            }
            ImpactMaterial::Glass => {
                level.play_sound(
                    host,
                    hit_sound,
                    event.at,
                    SoundSource::Players,
                    0.74,
                    (base_pitch * 1.18 + 0.05 * jitter).clamp(0.80, 1.85),
                );
                level.play_sound(
                    host,
                    break_sound,
                    event.at,
                    SoundSource::Players,
                    0.42,
                    1.36 + 0.06 * jitter,
                );
            }
            ImpactMaterial::Wood => {
                level.play_sound(
                    host,
                    place_sound,
                    event.at,
                    SoundSource::Players,
                    0.78,
                    (base_pitch * 0.92 + 0.04 * jitter).clamp(0.65, 1.45),
                );
                level.play_sound(
                    host,
                    break_sound,
                    event.at,
                    SoundSource::Players,
                    0.44,
                    (base_pitch * 1.28 + 0.05 * jitter).clamp(0.85, 1.75),
                );
            }
            ImpactMaterial::Earth => {
                level.play_sound(
                    host,
                    hit_sound,
                    event.at,
                    SoundSource::Players,
                    0.86,
                    (base_pitch * 0.90 + 0.04 * jitter).clamp(0.60, 1.35),
                );
                level.play_sound(
                    host,
                    fall_sound,
                    event.at,
                    SoundSource::Players,
                    0.54,
                    (base_pitch * 0.78 + 0.03 * jitter).clamp(0.55, 1.15),
                );
            }
            ImpactMaterial::Soft => {
                level.play_sound(
                    host,
                    hit_sound,
                    event.at,
                    SoundSource::Players,
                    0.70,
                    (base_pitch * 0.92 + 0.04 * jitter).clamp(0.60, 1.35),
                );
                level.play_sound(
                    host,
                    fall_sound,
                    event.at,
                    SoundSource::Players,
                    0.38,
                    (base_pitch * 0.84 + 0.03 * jitter).clamp(0.55, 1.20),
                );
            }
            ImpactMaterial::Stone => {
                level.play_sound(
                    host,
                    hit_sound,
                    event.at,
                    SoundSource::Players,
                    0.82,
                    (base_pitch * 1.18 + 0.05 * jitter).clamp(0.75, 1.75),
                );
                level.play_sound(
                    host,
                    SND_RICOCHET_STONE_CHIP,
                    event.at,
                    SoundSource::Players,
                    0.24,
                    1.76 + 0.08 * jitter,
                );
            }
        }
    }

    if !shattered_now
        && !native_will_shatter
        && let Some(block_key) = BLOCK_PARTICLE_KEY.get().copied()
    {
        let chip_opts = ParticleOptions::simple(block_key);
        let (chip_spread, chip_speed, chip_count) = if underwater {
            ([0.05, 0.05, 0.05], 0.08, 7)
        } else if in_leaves {
            ([0.12, 0.12, 0.12], 0.16, 16)
        } else {
            match material {
                ImpactMaterial::Metal => ([0.04, 0.04, 0.04], 0.12, 6),
                ImpactMaterial::Glass | ImpactMaterial::Crystal => ([0.06, 0.06, 0.06], 0.18, 13),
                ImpactMaterial::Earth => ([0.065, 0.065, 0.065], 0.13, 14),
                ImpactMaterial::Wood => ([0.05, 0.05, 0.05], 0.14, 11),
                ImpactMaterial::Soft => ([0.05, 0.05, 0.05], 0.10, 9),
                ImpactMaterial::Stone => ([0.05, 0.05, 0.05], 0.15, 10),
            }
        };
        level.send_particles(
            host,
            &chip_opts,
            Some(hit_state),
            impact_fx_pos,
            chip_spread,
            chip_speed,
            chip_count,
        );
    }

    if underwater {
        if let Some(bubble) = BUBBLE_KEY.get().copied() {
            let bubble_opts = ParticleOptions::simple(bubble);
            level.send_particles(
                host,
                &bubble_opts,
                None,
                impact_fx_pos,
                [0.05, 0.05, 0.05],
                0.06,
                8,
            );
        }
        if let Some(pop) = BUBBLE_POP_KEY.get().copied() {
            let pop_opts = ParticleOptions::simple(pop);
            level.send_particles(
                host,
                &pop_opts,
                None,
                impact_fx_pos,
                [0.04, 0.04, 0.04],
                0.02,
                3,
            );
        }
    } else if shattered_now || native_will_shatter {
        if let Some(white_smoke) = WHITE_SMOKE_KEY.get().copied() {
            let smoke_opts = ParticleOptions::simple(white_smoke);
            level.send_particles(
                host,
                &smoke_opts,
                None,
                impact_fx_pos,
                [0.05, 0.05, 0.05],
                0.015,
                2,
            );
        }
    } else {
        match material {
            ImpactMaterial::Metal => {
                if let Some(crit) = CRIT_KEY.get().copied() {
                    let crit_opts = ParticleOptions::simple(crit);
                    level.send_particles(
                        host,
                        &crit_opts,
                        None,
                        impact_fx_pos,
                        [0.04, 0.04, 0.04],
                        0.26,
                        7,
                    );
                }
                if let Some(white_smoke) = WHITE_SMOKE_KEY.get().copied() {
                    let smoke_opts = ParticleOptions::simple(white_smoke);
                    level.send_particles(
                        host,
                        &smoke_opts,
                        None,
                        impact_fx_pos,
                        [0.035, 0.035, 0.035],
                        0.014,
                        2,
                    );
                }
            }
            ImpactMaterial::Crystal => {
                if let Some(crit) = CRIT_KEY.get().copied() {
                    let crit_opts = ParticleOptions::simple(crit);
                    level.send_particles(
                        host,
                        &crit_opts,
                        None,
                        impact_fx_pos,
                        [0.05, 0.05, 0.05],
                        0.18,
                        4,
                    );
                }
            }
            ImpactMaterial::Glass => {}
            ImpactMaterial::Wood => {
                if let Some(poof) = POOF_KEY.get().copied() {
                    let poof_opts = ParticleOptions::simple(poof);
                    level.send_particles(
                        host,
                        &poof_opts,
                        None,
                        impact_fx_pos,
                        [0.035, 0.035, 0.035],
                        0.020,
                        2,
                    );
                }
            }
            ImpactMaterial::Earth => {
                if let Some(poof) = POOF_KEY.get().copied() {
                    let poof_opts = ParticleOptions::simple(poof);
                    level.send_particles(
                        host,
                        &poof_opts,
                        None,
                        impact_fx_pos,
                        [0.05, 0.05, 0.05],
                        0.028,
                        4,
                    );
                }
            }
            ImpactMaterial::Soft => {
                if let Some(poof) = POOF_KEY.get().copied() {
                    let poof_opts = ParticleOptions::simple(poof);
                    level.send_particles(
                        host,
                        &poof_opts,
                        None,
                        impact_fx_pos,
                        [0.03, 0.03, 0.03],
                        0.016,
                        1,
                    );
                }
            }
            ImpactMaterial::Stone => {
                if let Some(crit) = CRIT_KEY.get().copied() {
                    let crit_opts = ParticleOptions::simple(crit);
                    level.send_particles(
                        host,
                        &crit_opts,
                        None,
                        impact_fx_pos,
                        [0.03, 0.03, 0.03],
                        0.18,
                        2,
                    );
                }
                if let Some(poof) = POOF_KEY.get().copied() {
                    let poof_opts = ParticleOptions::simple(poof);
                    level.send_particles(
                        host,
                        &poof_opts,
                        None,
                        impact_fx_pos,
                        [0.035, 0.035, 0.035],
                        0.024,
                        3,
                    );
                }
                if let Some(smoke) = SMOKE_KEY.get().copied() {
                    let smoke_opts = ParticleOptions::simple(smoke);
                    level.send_particles(
                        host,
                        &smoke_opts,
                        None,
                        impact_fx_pos,
                        [0.045, 0.045, 0.045],
                        0.012,
                        3,
                    );
                }
            }
        }
    }

    if suppress_decal {
        return;
    }

    let Some(chan) = BULLET_IMPACT_SYNC.get() else {
        return;
    };
    let to_dm = |world: f64, block: i32| -> VarInt {
        let rel = ((world - f64::from(block)) * 10_000.0)
            .round()
            .clamp(-20_000.0, 30_000.0) as i32;
        VarInt(rel)
    };
    let payload = BulletImpactSync {
        dimension: event.dimension.to_owned(),
        block_x: VarInt(event.block[0]),
        block_y: VarInt(event.block[1]),
        block_z: VarInt(event.block[2]),
        face_x: VarInt(face[0]),
        face_y: VarInt(face[1]),
        face_z: VarInt(face[2]),
        rel_x_dm: to_dm(event.at[0], event.block[0]),
        rel_y_dm: to_dm(event.at[1], event.block[1]),
        rel_z_dm: to_dm(event.at[2], event.block[2]),
    };

    if let Some(players) = level.online_players(host) {
        for player in players {
            let near = player.entity().position(host).is_some_and(|p| {
                let dx = p[0] - event.at[0];
                let dy = p[1] - event.at[1];
                let dz = p[2] - event.at[2];
                dx * dx + dy * dy + dz * dz <= 80.0 * 80.0
            });
            if near {
                let _ = chan.send(host, player, &payload);
            }
        }
    }
}

pub(crate) fn on_bullet_impact_sync(
    value: BulletImpactSync,
    cx: &mut dyn IPayloadContext,
) -> Result<(), String> {
    let block = [value.block_x.0, value.block_y.0, value.block_z.0];
    let face = normalize_cardinal_face([value.face_x.0, value.face_y.0, value.face_z.0]);
    let local_hit = [
        f64::from(value.rel_x_dm.0) * 1e-4,
        f64::from(value.rel_y_dm.0) * 1e-4,
        f64::from(value.rel_z_dm.0) * 1e-4,
    ];
    let at = [
        f64::from(block[0]) + local_hit[0],
        f64::from(block[1]) + local_hit[1],
        f64::from(block[2]) + local_hit[2],
    ];

    let light_pos = [block[0] + face[0], block[1] + face[1], block[2] + face[2]];
    let mut light = LightCoords::new(0, 15).unwrap_or(LightCoords::FULL_BRIGHT);
    if let Ok(host) = cx.client_host() {
        let collision = host.block_collision(block);
        if !face_supports_bullet_decal(&collision, local_hit, face, BULLET_HOLE_MIN_FACE_SPAN) {
            return Ok(());
        }
        light = sample_world_light(host, &value.dimension, light_pos);
    }

    let max_holes = crate::settings::client_settings().max_bullet_holes;
    if max_holes == 0 {
        world_effects().lock().unwrap().holes.clear();
        return Ok(());
    }

    let mut fx = world_effects().lock().unwrap();
    if !value.dimension.is_empty() && fx.active_dimension != value.dimension {
        fx.active_dimension = value.dimension.clone();
        fx.holes.clear();
        fx.casings.clear();
    }
    while fx.holes.len() >= max_holes {
        fx.holes.remove(0);
    }
    let roll_quarter = ((fx.next_unit_f32() * 4.0) as u8) & 3;
    let slot_bias = fx.next_slot_bias & 7;
    fx.next_slot_bias = fx.next_slot_bias.wrapping_add(1);

    let dim = if value.dimension.is_empty() {
        fx.active_dimension.clone()
    } else {
        value.dimension
    };
    fx.holes.push(BulletHoleDecal {
        dimension: dim,
        block,
        face,
        at,
        roll_quarter,
        slot_bias,
        age: 0.0,
        light,
    });
    Ok(())
}

pub(crate) fn spawn_spent_casing_from_camera(host: &dyn ClientHost, _aim: f32) {
    let Some(cam) = host.camera_entity() else {
        return;
    };
    let Some(eye) = cam.eye_position(host) else {
        return;
    };
    let Some([yaw_deg, pitch_deg]) = cam.rotation(host) else {
        return;
    };

    let yaw_rad = f64::from(yaw_deg).to_radians();
    let pitch_rad = f64::from(pitch_deg).to_radians();
    let (sin_y, cos_y) = yaw_rad.sin_cos();
    let (sin_p, cos_p) = pitch_rad.sin_cos();

    let forward = [-sin_y * cos_p, -sin_p, cos_y * cos_p];
    let up = [-sin_y * sin_p, cos_p, cos_y * sin_p];
    let right = [-cos_y, 0.0, -sin_y];

    let first_person = host.camera_type() == CameraType::FirstPerson && holding_rifle_recently();
    let (port_right, port_up, port_fwd) = if first_person {
        // Exact 0-pixel handoff from `EJECTOR_HANDOFF_TEXELS` (`trava_dorsal` top carrier port):
        // converts 1st-person 70° hand-pass view coordinates `(ex, ey, ez)` into `world_fov`
        // world space at `target_fwd = 0.72` blocks in front of the eye.
        let ex = f64::from(f32::from_bits(LAST_EJECTOR_VIEW_X.load(Ordering::Relaxed)));
        let ey = f64::from(f32::from_bits(LAST_EJECTOR_VIEW_Y.load(Ordering::Relaxed)));
        let ez = f64::from(f32::from_bits(LAST_EJECTOR_VIEW_Z.load(Ordering::Relaxed)));
        let fwd_1p = (-ez).clamp(0.35, 2.50);
        let world_fov = f64::from(
            f32::from_bits(LAST_WORLD_FOV_BITS.load(Ordering::Relaxed)).clamp(20.0, 150.0),
        );
        let fov_ratio = (world_fov.to_radians() * 0.5).tan() / (35.0_f64.to_radians()).tan();
        let target_fwd = 0.72_f64;
        let ray_scale = target_fwd / fwd_1p;
        (
            (ex * fov_ratio * ray_scale).clamp(-1.10, 1.10),
            (ey * fov_ratio * ray_scale).clamp(-1.10, 1.10),
            target_fwd,
        )
    } else {
        // `F5` third-person held receiver top carrier position.
        (0.26, -0.26, 0.56)
    };

    let origin = [
        eye[0] + right[0] * port_right + up[0] * port_up + forward[0] * port_fwd,
        eye[1] + right[1] * port_right + up[1] * port_up + forward[1] * port_fwd,
        eye[2] + right[2] * port_right + up[2] * port_up + forward[2] * port_fwd,
    ];

    let mut fx = world_effects().lock().unwrap();
    let r0 = f64::from(fx.next_unit_f32());
    let r1 = f64::from(fx.next_unit_f32());
    let r2 = f64::from(fx.next_unit_f32());
    let spin_sign = if fx.next_unit_f32() >= 0.5 { 1.0 } else { -1.0 };

    // Authentic Winchester 1873 top-carrier ejection: pops up out of `trava_dorsal` and arcs
    // off to the right/rear away from the barrel & iron sights.
    let v_right = 0.92 + 0.38 * r0;
    let v_up = 1.55 + 0.38 * r1;
    let v_fwd = -0.18 + 0.24 * r2;
    let vel = [
        right[0] * v_right + up[0] * v_up + forward[0] * v_fwd,
        right[1] * v_right + up[1] * v_up + forward[1] * v_fwd,
        right[2] * v_right + up[2] * v_up + forward[2] * v_fwd,
    ];

    let dim = fx.active_dimension.clone();
    let block_pos = [
        origin[0].floor() as i32,
        origin[1].floor() as i32,
        origin[2].floor() as i32,
    ];
    let light = sample_world_light(host, &dim, block_pos);

    let max_casings = crate::settings::client_settings().max_casings;
    if max_casings == 0 {
        fx.casings.clear();
        return;
    }
    while fx.casings.len() >= max_casings {
        fx.casings.remove(0);
    }
    // Start aligned with the barrel & carrier slot (`local -Z` hollow mouth pointing forward,
    // tilted `+18°` upward matching Stage 1's exit angle) before tumbling end-over-end.
    let init_yaw = 180.0 - yaw_deg - 8.0;
    let init_pitch = -pitch_deg - 18.0;
    let pitch_vel = -(460.0 + 240.0 * fx.next_unit_f32());
    let yaw_vel = (210.0 + 160.0 * fx.next_unit_f32()) * spin_sign;
    let roll_vel = 560.0 + 240.0 * fx.next_unit_f32();

    fx.casings.push(SpentCasing {
        dimension: dim,
        pos: origin,
        vel,
        yaw_deg: init_yaw,
        pitch_deg: init_pitch,
        roll_deg: 24.0,
        yaw_vel,
        pitch_vel,
        roll_vel,
        bounces: 0,
        settled: false,
        age: 0.0,
        light,
    });
}

/// Spawns a 3D spinning/bouncing `.44-40` brass shell casing from a nearby remote player's
/// held Winchester top-carrier port so multiplayer observers see and hear brass clinking on the ground.
pub(crate) fn spawn_spent_casing_from_remote_actor(host: &dyn ClientHost, entity_id: u32) {
    let cfg = crate::settings::client_settings();
    if !cfg.casing_ejection || cfg.max_casings == 0 {
        return;
    }
    let Some(ent) = host.entity(entity_id) else {
        return;
    };
    let Some(eye) = ent.eye_position(host) else {
        return;
    };
    let [yaw_deg, pitch_deg] = ent.rotation(host).unwrap_or([0.0, 0.0]);

    let yaw_rad = f64::from(yaw_deg).to_radians();
    let pitch_rad = f64::from(pitch_deg).to_radians();
    let (sin_y, cos_y) = yaw_rad.sin_cos();
    let (sin_p, cos_p) = pitch_rad.sin_cos();

    let forward = [-sin_y * cos_p, -sin_p, cos_y * cos_p];
    let up = [-sin_y * sin_p, cos_p, cos_y * sin_p];
    let right = [-cos_y, 0.0, -sin_y];

    let origin = [
        eye[0] + right[0] * 0.26 + up[0] * -0.26 + forward[0] * 0.56,
        eye[1] + right[1] * 0.26 + up[1] * -0.26 + forward[1] * 0.56,
        eye[2] + right[2] * 0.26 + up[2] * -0.26 + forward[2] * 0.56,
    ];

    let mut fx = world_effects().lock().unwrap();
    let r0 = f64::from(fx.next_unit_f32());
    let r1 = f64::from(fx.next_unit_f32());
    let r2 = f64::from(fx.next_unit_f32());
    let spin_sign = if fx.next_unit_f32() >= 0.5 { 1.0 } else { -1.0 };

    let v_right = 0.92 + 0.38 * r0;
    let v_up = 1.55 + 0.38 * r1;
    let v_fwd = -0.18 + 0.24 * r2;
    let vel = [
        right[0] * v_right + up[0] * v_up + forward[0] * v_fwd,
        right[1] * v_right + up[1] * v_up + forward[1] * v_fwd,
        right[2] * v_right + up[2] * v_up + forward[2] * v_fwd,
    ];

    let dim = fx.active_dimension.clone();
    let block_pos = [
        origin[0].floor() as i32,
        origin[1].floor() as i32,
        origin[2].floor() as i32,
    ];
    let light = sample_world_light(host, &dim, block_pos);

    while fx.casings.len() >= cfg.max_casings {
        fx.casings.remove(0);
    }
    let init_yaw = 180.0 - yaw_deg - 8.0;
    let init_pitch = -pitch_deg - 18.0;
    let pitch_vel = -(460.0 + 240.0 * fx.next_unit_f32());
    let yaw_vel = (210.0 + 160.0 * fx.next_unit_f32()) * spin_sign;
    let roll_vel = 560.0 + 240.0 * fx.next_unit_f32();

    fx.casings.push(SpentCasing {
        dimension: dim,
        pos: origin,
        vel,
        yaw_deg: init_yaw,
        pitch_deg: init_pitch,
        roll_deg: 24.0,
        yaw_vel,
        pitch_vel,
        roll_vel,
        bounces: 0,
        settled: false,
        age: 0.0,
        light,
    });
}

fn floor_top_at(host: &dyn ClientHost, x: f64, y: f64, z: f64) -> Option<f64> {
    let bx = x.floor() as i32;
    let bz = z.floor() as i32;
    let base_y = y.floor() as i32;
    let lx = x - f64::from(bx);
    let lz = z - f64::from(bz);
    let pad = 0.025;

    let mut highest: Option<f64> = None;
    for by in [base_y - 1, base_y] {
        for b in host.block_collision([bx, by, bz]) {
            if lx >= b[0] - pad && lx <= b[3] + pad && lz >= b[2] - pad && lz <= b[5] + pad {
                let top = f64::from(by) + b[4];
                if top <= y + 0.22 {
                    highest = Some(highest.map_or(top, |cur| cur.max(top)));
                }
            }
        }
    }
    highest
}

fn hits_solid_wall(host: &dyn ClientHost, x: f64, y: f64, z: f64) -> bool {
    let bx = x.floor() as i32;
    let by = y.floor() as i32;
    let bz = z.floor() as i32;
    let lx = x - f64::from(bx);
    let ly = y - f64::from(by);
    let lz = z - f64::from(bz);
    host.block_collision([bx, by, bz]).into_iter().any(|b| {
        lx >= b[0] && lx <= b[3] && ly >= b[1] + 0.04 && ly <= b[4] - 0.04 && lz >= b[2] && lz <= b[5]
    })
}

pub(crate) fn step_world_effects(host: &dyn ClientHost, dt: f32) {
    let cfg = crate::settings::client_settings();
    let mut clink_sounds: Vec<([f64; 3], f32, f32)> = Vec::new();
    {
        let mut fx = world_effects().lock().unwrap();
        if fx.holes.is_empty() && fx.casings.is_empty() {
            return;
        }

        if cfg.max_bullet_holes == 0 {
            fx.holes.clear();
        } else if fx.holes.len() > cfg.max_bullet_holes {
            let drop_n = fx.holes.len() - cfg.max_bullet_holes;
            fx.holes.drain(0..drop_n);
        }
        for hole in &mut fx.holes {
            hole.age += dt;
        }
        fx.holes.retain(|h| h.age < cfg.bullet_hole_lifetime_secs);

        fx.prune_timer += dt;
        let refresh_world = fx.prune_timer >= 0.25;
        if refresh_world {
            fx.prune_timer = 0.0;
            fx.holes
                .retain(|h| !host.block_collision(h.block).is_empty());
            for hole in &mut fx.holes {
                let lit_pos = [
                    hole.block[0] + hole.face[0],
                    hole.block[1] + hole.face[1],
                    hole.block[2] + hole.face[2],
                ];
                hole.light = sample_world_light(host, &hole.dimension, lit_pos);
            }
        }

        let steps = ((dt / 0.022).ceil() as u32).clamp(1, 3);
        let h = dt / (steps as f32);
        let h64 = f64::from(h);

        let mut casings = core::mem::take(&mut fx.casings);
        for casing in &mut casings {
            casing.age += dt;
            if casing.settled {
                if refresh_world {
                    if floor_top_at(host, casing.pos[0], casing.pos[1] + 0.04, casing.pos[2])
                        .is_none()
                    {
                        casing.settled = false;
                    } else {
                        let bp = [
                            casing.pos[0].floor() as i32,
                            casing.pos[1].floor() as i32,
                            casing.pos[2].floor() as i32,
                        ];
                        casing.light = sample_world_light(host, &casing.dimension, bp);
                    }
                }
                continue;
            }

            let casing_level = host.level(&casing.dimension);
            for _ in 0..steps {
                let in_water = casing_level
                    .is_some_and(|lvl| is_point_in_water(host, lvl, casing.pos));
                if in_water {
                    casing.vel[1] =
                        (casing.vel[1] - CASING_WATER_GRAVITY * h64).max(CASING_WATER_MAX_SINK);
                    let water_drag = (1.0 - 7.5 * h64).clamp(0.72, 1.0);
                    casing.vel[0] *= water_drag;
                    casing.vel[2] *= water_drag;
                    let spin_drag = (1.0 - 5.5 * h).clamp(0.75, 1.0);
                    casing.yaw_vel *= spin_drag;
                    casing.pitch_vel *= spin_drag;
                    casing.roll_vel *= spin_drag;
                } else {
                    casing.vel[1] -= 15.5 * h64;
                    let drag = (1.0 - 0.85 * h64).clamp(0.90, 1.0);
                    casing.vel[0] *= drag;
                    casing.vel[2] *= drag;
                }

                let next_x = casing.pos[0] + casing.vel[0] * h64;
                let next_z = casing.pos[2] + casing.vel[2] * h64;
                if hits_solid_wall(host, next_x, casing.pos[1] + 0.03, casing.pos[2]) {
                    casing.vel[0] = -casing.vel[0] * if in_water { 0.18 } else { 0.42 };
                } else {
                    casing.pos[0] = next_x;
                }
                if hits_solid_wall(host, casing.pos[0], casing.pos[1] + 0.03, next_z) {
                    casing.vel[2] = -casing.vel[2] * if in_water { 0.18 } else { 0.42 };
                } else {
                    casing.pos[2] = next_z;
                }

                let prev_y = casing.pos[1];
                let next_y = prev_y + casing.vel[1] * h64;
                if casing.vel[1] <= 0.0
                    && let Some(floor_y) = floor_top_at(host, casing.pos[0], prev_y, casing.pos[2])
                    && next_y <= floor_y + CASING_REST_RADIUS
                {
                    casing.pos[1] = floor_y + CASING_REST_RADIUS;
                    let impact_speed = (-casing.vel[1]) as f32;
                    if in_water {
                        if impact_speed > 0.35 && casing.bounces == 0 {
                            let pitch_var = 0.72 + 0.08 * fx.next_unit_f32();
                            clink_sounds.push((casing.pos, 0.24, pitch_var));
                        }
                        casing.vel = [0.0; 3];
                        casing.pitch_deg = 0.0;
                        casing.pitch_vel = 0.0;
                        casing.yaw_vel = 0.0;
                        casing.roll_vel = 0.0;
                        casing.settled = true;
                        break;
                    } else if impact_speed > 0.58 && casing.bounces < 4 {
                        casing.vel[1] = -casing.vel[1] * 0.38;
                        casing.vel[0] *= 0.62;
                        casing.vel[2] *= 0.62;
                        casing.pitch_deg *= 0.25;
                        casing.pitch_vel *= 0.28;
                        casing.yaw_vel *= 0.55;
                        casing.roll_vel *= 0.65;
                        if casing.bounces < 2 {
                            let vol = if casing.bounces == 0 { 0.68 } else { 0.34 };
                            let pitch_var = 0.94 + 0.12 * fx.next_unit_f32();
                            clink_sounds.push((casing.pos, vol, pitch_var));
                        }
                        casing.bounces += 1;
                    } else {
                        casing.vel = [0.0; 3];
                        casing.pitch_deg = 0.0;
                        casing.pitch_vel = 0.0;
                        casing.yaw_vel = 0.0;
                        casing.roll_vel = 0.0;
                        casing.settled = true;
                        break;
                    }
                } else {
                    casing.pos[1] = next_y;
                }

                casing.yaw_deg = (casing.yaw_deg + casing.yaw_vel * h).rem_euclid(360.0);
                casing.pitch_deg = (casing.pitch_deg + casing.pitch_vel * h).rem_euclid(360.0);
                casing.roll_deg = (casing.roll_deg + casing.roll_vel * h).rem_euclid(360.0);
            }

            let bp = [
                casing.pos[0].floor() as i32,
                casing.pos[1].floor() as i32,
                casing.pos[2].floor() as i32,
            ];
            casing.light = sample_world_light(host, &casing.dimension, bp);
        }

        if cfg.max_casings == 0 {
            casings.clear();
        } else {
            casings.retain(|c| c.age < cfg.casing_lifetime_secs && c.pos[1] > -128.0);
            if casings.len() > cfg.max_casings {
                let drop_n = casings.len() - cfg.max_casings;
                casings.drain(0..drop_n);
            }
        }
        fx.casings = casings;
    }

    for (pos, vol, pitch_mul) in clink_sounds {
        play_local_positional_foley(host, SND_SHELL_DROP, pos, vol, pitch_mul, 0);
    }
}

/// Builds a right-handed (`det > 0`) row-major `3x4` local transform around model center
/// `[0.5, 0.5, 0.5]` from scaled basis columns `(col_x, col_y, col_z)`.
fn centered_row_major_3x4(col_x: [f32; 3], col_y: [f32; 3], col_z: [f32; 3]) -> [f32; 12] {
    let tx = -0.5 * (col_x[0] + col_y[0] + col_z[0]);
    let ty = -0.5 * (col_x[1] + col_y[1] + col_z[1]);
    let tz = -0.5 * (col_x[2] + col_y[2] + col_z[2]);
    [
        col_x[0], col_y[0], col_z[0], tx, col_x[1], col_y[1], col_z[1], ty, col_x[2], col_y[2],
        col_z[2], tz,
    ]
}

fn bullet_hole_instance_transform(face: [i32; 3], roll_quarter: u8, scale: f32) -> [f32; 12] {
    let f = normalize_cardinal_face(face);
    // Right-handed orthonormal basis `(u, v, n)` with `u × v == n` (`det == +1`).
    let (u0, v0, n) = match f {
        [0, 0, 1] => (
            [1.0_f32, 0.0, 0.0],
            [0.0_f32, 1.0, 0.0],
            [0.0_f32, 0.0, 1.0],
        ),
        [0, 0, -1] => (
            [-1.0_f32, 0.0, 0.0],
            [0.0_f32, 1.0, 0.0],
            [0.0_f32, 0.0, -1.0],
        ),
        [1, 0, 0] => (
            [0.0_f32, 0.0, -1.0],
            [0.0_f32, 1.0, 0.0],
            [1.0_f32, 0.0, 0.0],
        ),
        [-1, 0, 0] => (
            [0.0_f32, 0.0, 1.0],
            [0.0_f32, 1.0, 0.0],
            [-1.0_f32, 0.0, 0.0],
        ),
        [0, 1, 0] => (
            [1.0_f32, 0.0, 0.0],
            [0.0_f32, 0.0, -1.0],
            [0.0_f32, 1.0, 0.0],
        ),
        _ => (
            [1.0_f32, 0.0, 0.0],
            [0.0_f32, 0.0, 1.0],
            [0.0_f32, -1.0, 0.0],
        ),
    };

    let (u, v) = match roll_quarter & 3 {
        0 => (u0, v0),
        1 => (v0, [-u0[0], -u0[1], -u0[2]]),
        2 => ([-u0[0], -u0[1], -u0[2]], [-v0[0], -v0[1], -v0[2]]),
        _ => ([-v0[0], -v0[1], -v0[2]], u0),
    };

    let s = scale.max(0.01);
    let sn = (scale * 0.25).max(0.01);
    centered_row_major_3x4(
        [u[0] * s, u[1] * s, u[2] * s],
        [v[0] * s, v[1] * s, v[2] * s],
        [n[0] * sn, n[1] * sn, n[2] * sn],
    )
}

fn casing_instance_transform(
    yaw_deg: f32,
    pitch_deg: f32,
    roll_deg: f32,
    scale: f32,
) -> [f32; 12] {
    let (sy, cy) = yaw_deg.to_radians().sin_cos();
    let (sp, cp) = pitch_deg.to_radians().sin_cos();
    let (sr, cr) = roll_deg.to_radians().sin_cos();

    // Proper rotation `R = R_y(yaw) * R_x(pitch) * R_z(roll)` (`det(R) == +1`).
    // When settled (`pitch_deg == 0`), local Z cylinder axis `[sy * cp, -sp, cy * cp]`
    // lies cleanly in the horizontal XZ plane (`y == 0`).
    let s = scale.max(0.01);
    let col_x = [
        (cy * cr + sy * sp * sr) * s,
        (cp * sr) * s,
        (-sy * cr + cy * sp * sr) * s,
    ];
    let col_y = [
        (-cy * sr + sy * sp * cr) * s,
        (cp * cr) * s,
        (sy * sr + cy * sp * cr) * s,
    ];
    let col_z = [(sy * cp) * s, (-sp) * s, (cy * cp) * s];
    centered_row_major_3x4(col_x, col_y, col_z)
}

pub(crate) fn submit_world_effects(draw: &mut dyn WorldDrawContext) {
    let Some(models) = *RIFLE_MODELS.lock().unwrap() else {
        return;
    };
    let fx = world_effects().lock().unwrap();
    if fx.holes.is_empty() && fx.casings.is_empty() {
        return;
    }

    if !fx.holes.is_empty() {
        let mut hole_instances = Vec::with_capacity(fx.holes.len());
        for hole in &fx.holes {
            let rem = (BULLET_HOLE_LIFETIME_SECS - hole.age).max(0.0);
            let fade_scale = if rem < 2.5 { rem / 2.5 } else { 1.0 };
            let scale = BULLET_HOLE_BASE_SCALE * fade_scale;
            if scale <= 0.02 {
                continue;
            }
            let lift = 0.004 + f64::from(hole.slot_bias) * 0.00045;
            let origin = [
                hole.at[0] + f64::from(hole.face[0]) * lift,
                hole.at[1] + f64::from(hole.face[1]) * lift,
                hole.at[2] + f64::from(hole.face[2]) * lift,
            ];
            let transform = bullet_hole_instance_transform(hole.face, hole.roll_quarter, scale);
            hole_instances.push(ModelInstance::new(
                origin,
                transform,
                0xFF_FF_FF_FF,
                hole.light,
                Overlay::NONE,
            ));
        }
        if !hole_instances.is_empty() {
            let _ = draw.draw_model_instances(models.bullet_hole, &hole_instances);
        }
    }

    if !fx.casings.is_empty() {
        let mut casing_instances = Vec::with_capacity(fx.casings.len());
        for casing in &fx.casings {
            let rem = (CASING_LIFETIME_SECS - casing.age).max(0.0);
            let fade_scale = if rem < 1.5 { rem / 1.5 } else { 1.0 };
            let scale = CASING_BASE_SCALE * fade_scale;
            if scale <= 0.03 {
                continue;
            }
            let transform = casing_instance_transform(
                casing.yaw_deg,
                casing.pitch_deg,
                casing.roll_deg,
                scale,
            );
            casing_instances.push(ModelInstance::new(
                casing.pos,
                transform,
                0xFF_FF_FF_FF,
                casing.light,
                Overlay::NONE,
            ));
        }
        if !casing_instances.is_empty() {
            let _ = draw.draw_model_instances(models.casing, &casing_instances);
        }
    }
}

#[cfg(test)]
mod world_effects_tests {
    use super::*;

    fn det3(m: [f32; 12]) -> f32 {
        let (a, b, c) = (m[0], m[1], m[2]);
        let (d, e, f) = (m[4], m[5], m[6]);
        let (g, h, i) = (m[8], m[9], m[10]);
        a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g)
    }

    fn apply3x4(m: [f32; 12], p: [f32; 3]) -> [f32; 3] {
        [
            m[0] * p[0] + m[1] * p[1] + m[2] * p[2] + m[3],
            m[4] * p[0] + m[5] * p[1] + m[6] * p[2] + m[7],
            m[8] * p[0] + m[9] * p[1] + m[10] * p[2] + m[11],
        ]
    }

    #[test]
    fn bullet_hole_transforms_preserve_winding_and_center() {
        let faces = [
            [0, 0, 1],
            [0, 0, -1],
            [1, 0, 0],
            [-1, 0, 0],
            [0, 1, 0],
            [0, -1, 0],
        ];
        for face in faces {
            for roll in 0..4 {
                let m = bullet_hole_instance_transform(face, roll, BULLET_HOLE_BASE_SCALE);
                assert!(det3(m) > 0.0, "face {face:?} roll {roll} inverted winding");
                let center = apply3x4(m, [0.5, 0.5, 0.5]);
                assert!(
                    center[0].abs() < 1e-6 && center[1].abs() < 1e-6 && center[2].abs() < 1e-6
                );
            }
        }
    }

    #[test]
    fn settled_casing_transform_keeps_cylinder_horizontal() {
        for yaw in [0.0_f32, 37.5, 142.0, 289.0] {
            let m = casing_instance_transform(yaw, 0.0, 65.0, CASING_BASE_SCALE);
            assert!(det3(m) > 0.0);
            let center = apply3x4(m, [0.5, 0.5, 0.5]);
            assert!(center[0].abs() < 1e-6 && center[1].abs() < 1e-6 && center[2].abs() < 1e-6);
            // Local Z axis (mouth-to-rim cylinder axis) must lie flat in XZ (`m[6] == 0`).
            assert!(m[6].abs() < 1e-6);
        }
    }

    #[test]
    fn impact_material_classifies_all_vanilla_families() {
        assert_eq!(
            classify_impact_material(
                "minecraft:cut_copper",
                "minecraft:block.copper.hit",
                "minecraft:block.copper.break"
            ),
            ImpactMaterial::Metal
        );
        assert_eq!(
            classify_impact_material(
                "minecraft:anvil",
                "minecraft:block.anvil.hit",
                "minecraft:block.anvil.destroy"
            ),
            ImpactMaterial::Metal
        );
        assert!(is_heavy_metal_surface(
            "minecraft:anvil",
            "minecraft:block.anvil.hit"
        ));
        assert!(!is_heavy_metal_surface(
            "minecraft:lantern",
            "minecraft:block.lantern.hit"
        ));
        assert_eq!(
            classify_impact_material(
                "minecraft:amethyst_block",
                "minecraft:block.amethyst_block.hit",
                "minecraft:block.amethyst_block.break"
            ),
            ImpactMaterial::Crystal
        );
        assert_eq!(
            classify_impact_material(
                "minecraft:cyan_stained_glass",
                "minecraft:block.glass.hit",
                "minecraft:block.glass.break"
            ),
            ImpactMaterial::Glass
        );
        assert_eq!(
            classify_impact_material(
                "minecraft:spruce_planks",
                "minecraft:block.wood.hit",
                "minecraft:block.wood.break"
            ),
            ImpactMaterial::Wood
        );
        assert_eq!(
            classify_impact_material(
                "minecraft:sand",
                "minecraft:block.sand.hit",
                "minecraft:block.sand.break"
            ),
            ImpactMaterial::Earth
        );
        assert_eq!(
            classify_impact_material(
                "minecraft:target",
                "minecraft:block.grass.hit",
                "minecraft:block.grass.break"
            ),
            ImpactMaterial::Soft
        );
        assert_eq!(
            classify_impact_material(
                "minecraft:deepslate_bricks",
                "minecraft:block.deepslate_bricks.hit",
                "minecraft:block.deepslate_bricks.break"
            ),
            ImpactMaterial::Stone
        );
    }

    #[test]
    fn state_property_helpers_parse_and_update_cleanly() {
        let s = "minecraft:red_candle[candles=3,lit=true,waterlogged=false]";
        assert_eq!(parse_block_state_id(s), "minecraft:red_candle");
        assert_eq!(parse_state_property(s, "lit"), Some("true"));
        assert_eq!(parse_state_property(s, "candles"), Some("3"));
        assert_eq!(
            replace_state_property(s, "lit", "false").as_deref(),
            Some("minecraft:red_candle[candles=3,lit=false,waterlogged=false]")
        );
        assert_eq!(replace_state_property(s, "lit", "true"), None);
    }

    #[test]
    fn bell_and_target_reactive_rules_match_ballistics() {
        let ceiling_bell = "minecraft:bell[attachment=ceiling,facing=north,powered=false]";
        // Side shot on ceiling bell -> Vanilla already rings it.
        assert!(vanilla_bell_will_ring(ceiling_bell, [0, 0, 1], 0.50));
        // Upward shot from village street (`face = [0, -1, 0]`) -> Vanilla misses it, our fallback rings!
        assert!(!vanilla_bell_will_ring(ceiling_bell, [0, -1, 0], 0.25));
        assert!(matches!(bell_swing_direction_3d(ceiling_bell, [0, -1, 0]), 2..=5));

        let wall_bell = "minecraft:bell[attachment=single_wall,facing=east,powered=false]";
        assert!(!vanilla_bell_will_ring(wall_bell, [1, 0, 0], 0.45));
        assert!(vanilla_bell_will_ring(wall_bell, [0, 0, 1], 0.45));
        assert!(matches!(bell_swing_direction_3d(wall_bell, [1, 0, 0]), 2 | 3));

        // Dead-center bullseye = 15, mid-ring = 8, outer corner = 1.
        assert_eq!(
            compute_target_bullseye_power([10, 64, 20], [0, 0, 1], [10.50, 64.50, 21.0]),
            15
        );
        assert_eq!(
            compute_target_bullseye_power([10, 64, 20], [0, 0, 1], [10.98, 64.98, 21.0]),
            1
        );
        let mid = compute_target_bullseye_power([10, 64, 20], [1, 0, 0], [11.0, 64.75, 20.50]);
        assert!((7..=9).contains(&mid));
    }

    #[test]
    fn fragile_blocks_and_decal_face_support_filter_correctly() {
        assert!(is_fragile_bullet_shatter_block("minecraft:glass_pane", false));
        assert!(is_fragile_bullet_shatter_block(
            "minecraft:amber_stained_glass_pane",
            false
        ));
        assert!(is_fragile_bullet_shatter_block(
            "minecraft:pointed_dripstone",
            false
        ));
        assert!(is_fragile_bullet_shatter_block("minecraft:potted_cactus", true));
        // Solid 1m³ glass blocks and budding amethyst stay intact.
        assert!(!is_fragile_bullet_shatter_block("minecraft:glass", false));
        assert!(!is_fragile_bullet_shatter_block(
            "minecraft:small_amethyst_bud",
            false
        ));

        let full_cube = [[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]];
        assert!(face_supports_bullet_decal(
            &full_cube,
            [0.5, 0.5, 1.0],
            [0, 0, 1],
            BULLET_HOLE_MIN_FACE_SPAN
        ));
        let fence_post = [[0.375, 0.0, 0.375, 0.625, 1.0, 0.625]];
        assert!(!face_supports_bullet_decal(
            &fence_post,
            [0.5, 0.5, 0.625],
            [0, 0, 1],
            BULLET_HOLE_MIN_FACE_SPAN
        ));
    }
}
