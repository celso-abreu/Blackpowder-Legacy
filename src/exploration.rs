//! Vanilla structure exploration loot (`container_loot_filled`) & custom advancement triggers.
//!
//! - Injects balanced `.44-40` ammunition caches and rare unloaded Winchester 1873 rifles (`0 / 8`)
//!   into Abandoned Mineshafts (including chest minecarts), Pillager Outposts, Woodland Mansions,
//!   plus occasional loose cartridges in underground Dungeons and Desert Pyramids.
//! - Drives mod-owned advancement criteria (`PlayerRef::trigger_criterion`) for frontier relics,
//!   full `8 / 8` tube reloads, submerged wet misfires, long-range bell ringing (`30m+`), candle
//!   snuffing, `50m+` Target bullseyes (`power == 15`), Illager bounty hunting, and `60m+`
//!   marksman kills.

use std::sync::{Arc, Mutex};

use mod_api::{
    ContainerLootFilledEvent, CriterionContext, CriterionTest, CriterionTestError, Host,
    Identifier, ItemStack, LevelHost, LevelRef, LivingDeathEvent, PlayerRef, ServerHost,
};

use crate::constants::*;
use crate::server::rounds;
use crate::{CARTRIDGE_KEY, RIFLE_KEY};

// ---------------------------------------------------------------------------
// Always-Pass Criterion Predicate for Server-Validated Mod Triggers
// ---------------------------------------------------------------------------

struct ValidatedPass;

impl CriterionTest for ValidatedPass {
    fn test(&self, _: &dyn CriterionContext) -> Result<bool, CriterionTestError> {
        Ok(true)
    }
}

pub(crate) fn grant_mod_criterion(host: &mut dyn ServerHost, player: PlayerRef, trigger_id: &str) {
    let Ok(id) = Identifier::parse(trigger_id) else {
        return;
    };
    let _ = player.trigger_criterion(host, &id, Arc::new(ValidatedPass));
}

// ---------------------------------------------------------------------------
// Structure Container Loot Injection
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StructureLootSpec {
    pub(crate) ammo_chance: f32,
    pub(crate) ammo_min: u32,
    pub(crate) ammo_max: u32,
    pub(crate) rifle_chance: f32,
    pub(crate) rifle_bonus_ammo_min: u32,
    pub(crate) rifle_bonus_ammo_max: u32,
    pub(crate) grants_relic_advancement: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RolledFrontierLoot {
    pub(crate) cartridges: u32,
    pub(crate) unloaded_rifle: bool,
}

pub(crate) fn loot_spec_for_table(table: &str) -> Option<StructureLootSpec> {
    match table {
        LOOT_ABANDONED_MINESHAFT => Some(StructureLootSpec {
            ammo_chance: 0.32,
            ammo_min: 3,
            ammo_max: 7,
            rifle_chance: 0.06,
            rifle_bonus_ammo_min: 2,
            rifle_bonus_ammo_max: 4,
            grants_relic_advancement: true,
        }),
        LOOT_PILLAGER_OUTPOST => Some(StructureLootSpec {
            ammo_chance: 0.42,
            ammo_min: 4,
            ammo_max: 9,
            rifle_chance: 0.10,
            rifle_bonus_ammo_min: 3,
            rifle_bonus_ammo_max: 5,
            grants_relic_advancement: true,
        }),
        LOOT_WOODLAND_MANSION => Some(StructureLootSpec {
            ammo_chance: 0.36,
            ammo_min: 4,
            ammo_max: 10,
            rifle_chance: 0.09,
            rifle_bonus_ammo_min: 3,
            rifle_bonus_ammo_max: 6,
            grants_relic_advancement: true,
        }),
        LOOT_SIMPLE_DUNGEON => Some(StructureLootSpec {
            ammo_chance: 0.16,
            ammo_min: 2,
            ammo_max: 5,
            rifle_chance: 0.0,
            rifle_bonus_ammo_min: 0,
            rifle_bonus_ammo_max: 0,
            grants_relic_advancement: false,
        }),
        LOOT_DESERT_PYRAMID => Some(StructureLootSpec {
            ammo_chance: 0.14,
            ammo_min: 2,
            ammo_max: 5,
            rifle_chance: 0.0,
            rifle_bonus_ammo_min: 0,
            rifle_bonus_ammo_max: 0,
            grants_relic_advancement: false,
        }),
        _ => None,
    }
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn fnv1a_bytes(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

fn unit_f32_from_u64(v: u64) -> f32 {
    ((v >> 40) as u32) as f32 * (1.0 / 16_777_216.0)
}

fn range_inclusive_u32(v: u64, min: u32, max: u32) -> u32 {
    if max <= min {
        return min;
    }
    let span = (max - min + 1) as u64;
    min + ((v >> 16) % span) as u32
}

pub(crate) fn compute_container_seed(
    table: &str,
    dimension: &str,
    origin: [f64; 3],
    entity: Option<u32>,
    drops_fingerprint: u64,
) -> u64 {
    let mut h = 0xCBF2_9CE4_8422_2325_u64;
    h = fnv1a_bytes(h, table.as_bytes());
    h = fnv1a_bytes(h, b"|");
    h = fnv1a_bytes(h, dimension.as_bytes());
    let qx = (origin[0] * 16.0).round() as i64 as u64;
    let qy = (origin[1] * 16.0).round() as i64 as u64;
    let qz = (origin[2] * 16.0).round() as i64 as u64;
    h = splitmix64(h ^ qx.rotate_left(11) ^ qy.rotate_left(27) ^ qz.rotate_left(43));
    h = splitmix64(h ^ u64::from(entity.unwrap_or(0)).wrapping_mul(0x9E37_79B9));
    splitmix64(h ^ drops_fingerprint)
}

pub(crate) fn roll_structure_loot(spec: StructureLootSpec, seed: u64) -> RolledFrontierLoot {
    let r0 = splitmix64(seed);
    let r1 = splitmix64(r0);
    let r2 = splitmix64(r1);
    let r3 = splitmix64(r2);

    let mut cartridges = if spec.ammo_chance > 0.0 && unit_f32_from_u64(r0) < spec.ammo_chance {
        range_inclusive_u32(r1, spec.ammo_min, spec.ammo_max)
    } else {
        0
    };

    let unloaded_rifle = spec.rifle_chance > 0.0 && unit_f32_from_u64(r2) < spec.rifle_chance;
    if unloaded_rifle && cartridges == 0 && spec.rifle_bonus_ammo_max > 0 {
        cartridges =
            range_inclusive_u32(r3, spec.rifle_bonus_ammo_min, spec.rifle_bonus_ammo_max);
    }

    RolledFrontierLoot {
        cartridges,
        unloaded_rifle,
    }
}

static PENDING_RELIC_FINDERS: Mutex<Vec<u128>> = Mutex::new(Vec::new());

pub(crate) fn on_container_loot_filled(
    _level: &dyn LevelHost,
    event: &mut ContainerLootFilledEvent<'_>,
) {
    let Some(spec) = loot_spec_for_table(event.context.table) else {
        return;
    };
    let Some(cartridge_key) = CARTRIDGE_KEY.get().copied() else {
        return;
    };
    let Some(rifle_key) = RIFLE_KEY.get().copied() else {
        return;
    };

    // Fingerprint the freshly rolled Vanilla stack counts & damage values so the container seed
    // varies naturally with the world's `random_sequence` as well as container coordinates.
    let mut fp = (event.drops.len() as u64).wrapping_mul(0x517C_C1B7_2722_0A95);
    for stack in event.drops.iter() {
        fp = splitmix64(
            fp ^ (u64::from(stack.count()) << 16) ^ u64::from(stack.damage()),
        );
    }
    let seed = compute_container_seed(
        event.context.table,
        event.context.dimension,
        event.context.origin,
        event.context.entity,
        fp,
    );
    let rolled = roll_structure_loot(spec, seed);

    if rolled.cartridges > 0
        && let Some(ammo_stack) = ItemStack::new(cartridge_key, rolled.cartridges)
    {
        event.drops.push(ammo_stack);
    }

    if rolled.unloaded_rifle
        && let Some(mut rifle_stack) = ItemStack::new(rifle_key, 1)
    {
        // `damage == 0` encodes `0 / 8` loaded rounds (`rounds(&stack) == 0`, hammer uncocked).
        rifle_stack.set_damage(0);
        debug_assert_eq!(rounds(&rifle_stack), 0);
        event.drops.push(rifle_stack);

        if spec.grants_relic_advancement
            && let Some(opener_uuid) = event.context.opener
        {
            let mut pending = PENDING_RELIC_FINDERS.lock().unwrap();
            if !pending.contains(&opener_uuid) && pending.len() < 32 {
                pending.push(opener_uuid);
            }
        }
    }
}

pub(crate) fn flush_pending_exploration_advancements(host: &mut dyn ServerHost) {
    let uuids: Vec<u128> = {
        let mut pending = PENDING_RELIC_FINDERS.lock().unwrap();
        if pending.is_empty() {
            return;
        }
        core::mem::take(&mut *pending)
    };
    for uuid in uuids {
        if let Some(player) = host.player(uuid) {
            grant_mod_criterion(host, player, TRIG_FRONTIER_RELIC);
        }
    }
}

// ---------------------------------------------------------------------------
// Shot Origin & Entity-Kill Ballistic Tracking for Advancements
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct RecentShot {
    bullet_entity_id: u32,
    shooter_entity_id: u32,
    shooter_uuid: u128,
    origin: [f64; 3],
    tick: u64,
}

#[derive(Clone, Copy)]
struct RecentEntityHit {
    victim_entity_id: u32,
    shooter_uuid: u128,
    distance_blocks: f64,
    tick: u64,
}

static RECENT_SHOTS: Mutex<Vec<RecentShot>> = Mutex::new(Vec::new());
static RECENT_ENTITY_HITS: Mutex<Vec<RecentEntityHit>> = Mutex::new(Vec::new());

pub(crate) fn distance_3d(a: [f64; 3], b: [f64; 3]) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

pub(crate) fn record_bullet_shot(
    bullet_entity_id: u32,
    shooter_entity_id: u32,
    shooter_uuid: u128,
    origin: [f64; 3],
    tick: u64,
) {
    let mut shots = RECENT_SHOTS.lock().unwrap();
    shots.retain(|s| tick.saturating_sub(s.tick) <= 200);
    if shots.len() >= 64 {
        shots.remove(0);
    }
    shots.push(RecentShot {
        bullet_entity_id,
        shooter_entity_id,
        shooter_uuid,
        origin,
        tick,
    });
}

/// Resolves the online shooter (`PlayerRef`) and exact 3D shot distance (`blocks`) for a bullet
/// impact owned by `owner_entity_id`.
pub(crate) fn resolve_shooter_and_distance(
    host: &dyn Host,
    level: LevelRef,
    owner_entity_id: u32,
    impact_at: [f64; 3],
) -> Option<(PlayerRef, f64)> {
    if owner_entity_id == 0 {
        return None;
    }
    let shooter = level
        .online_players(host)?
        .into_iter()
        .find(|p| p.entity().id() == owner_entity_id)?;

    let recorded_origin = RECENT_SHOTS
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|s| s.shooter_entity_id == owner_entity_id)
        .map(|s| s.origin);

    let origin = recorded_origin
        .or_else(|| shooter.entity().eye_position(host))
        .or_else(|| shooter.entity().position(host))?;

    Some((shooter, distance_3d(origin, impact_at)))
}

pub(crate) fn record_bullet_entity_hit(
    victim_entity_id: u32,
    shooter_uuid: u128,
    distance_blocks: f64,
    tick: u64,
) {
    let mut hits = RECENT_ENTITY_HITS.lock().unwrap();
    hits.retain(|h| tick.saturating_sub(h.tick) <= 40);
    if hits.len() >= 64 {
        hits.remove(0);
    }
    hits.push(RecentEntityHit {
        victim_entity_id,
        shooter_uuid,
        distance_blocks,
        tick,
    });
}

pub(crate) fn is_illager_kind(kind: &str) -> bool {
    matches!(
        kind,
        "minecraft:pillager"
            | "minecraft:vindicator"
            | "minecraft:evoker"
            | "minecraft:illusioner"
            | "minecraft:ravager"
    )
}

pub(crate) fn is_hostile_monster_kind(kind: &str) -> bool {
    if is_illager_kind(kind) {
        return true;
    }
    matches!(
        kind,
        "minecraft:blaze"
            | "minecraft:bogged"
            | "minecraft:breeze"
            | "minecraft:cave_spider"
            | "minecraft:creaking"
            | "minecraft:creeper"
            | "minecraft:drowned"
            | "minecraft:elder_guardian"
            | "minecraft:enderman"
            | "minecraft:endermite"
            | "minecraft:ghast"
            | "minecraft:guardian"
            | "minecraft:hoglin"
            | "minecraft:husk"
            | "minecraft:magma_cube"
            | "minecraft:phantom"
            | "minecraft:piglin_brute"
            | "minecraft:shulker"
            | "minecraft:silverfish"
            | "minecraft:skeleton"
            | "minecraft:slime"
            | "minecraft:spider"
            | "minecraft:stray"
            | "minecraft:vex"
            | "minecraft:warden"
            | "minecraft:witch"
            | "minecraft:wither"
            | "minecraft:wither_skeleton"
            | "minecraft:zoglin"
            | "minecraft:zombie"
            | "minecraft:zombie_villager"
            | "minecraft:zombified_piglin"
    )
}

pub(crate) fn handle_living_death_advancements(
    host: &mut dyn ServerHost,
    event: &LivingDeathEvent<'_>,
    now: u64,
) {
    // Match either by direct bullet entity ID (`source.direct`) or by the impact recorded in
    // the same tick on `event.entity`.
    let matched: Option<(u128, f64)> = {
        let direct_id = if event.source.direct > 0 {
            Some(event.source.direct as u32)
        } else {
            None
        };
        let from_hit = RECENT_ENTITY_HITS
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|h| h.victim_entity_id == event.entity && now.saturating_sub(h.tick) <= 3)
            .map(|h| (h.shooter_uuid, h.distance_blocks));

        if from_hit.is_some() {
            from_hit
        } else if let Some(bid) = direct_id {
            let shot = RECENT_SHOTS
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|s| s.bullet_entity_id == bid && now.saturating_sub(s.tick) <= 200)
                .copied();
            shot.and_then(|s| {
                let shooter = host.player(s.shooter_uuid)?;
                let victim_pos = event.source.position.or_else(|| shooter.entity().position(host))?;
                Some((s.shooter_uuid, distance_3d(s.origin, victim_pos)))
            })
        } else {
            None
        }
    };

    let Some((shooter_uuid, shot_dist)) = matched else {
        return;
    };
    let Some(shooter) = host.player(shooter_uuid) else {
        return;
    };

    if is_illager_kind(event.kind) {
        grant_mod_criterion(host, shooter, TRIG_SHERIFF);
    }
    if is_hostile_monster_kind(event.kind) && shot_dist >= ADV_LONG_SHOT_MIN_DIST {
        grant_mod_criterion(host, shooter, TRIG_LONG_SHOT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_structure_tables_match_expected_loot_profiles() {
        for table in [
            LOOT_ABANDONED_MINESHAFT,
            LOOT_PILLAGER_OUTPOST,
            LOOT_WOODLAND_MANSION,
        ] {
            let spec = loot_spec_for_table(table).expect(table);
            assert!(spec.ammo_chance > 0.25 && spec.ammo_chance < 0.50);
            assert!(spec.rifle_chance > 0.04 && spec.rifle_chance <= 0.12);
            assert!(spec.grants_relic_advancement);
        }

        // Secondary caches carry only `.44-40` cartridges, keeping rifles rare & thematic.
        for secondary in [LOOT_SIMPLE_DUNGEON, LOOT_DESERT_PYRAMID] {
            let spec = loot_spec_for_table(secondary).expect(secondary);
            assert!(spec.ammo_chance > 0.10);
            assert_eq!(spec.rifle_chance, 0.0);
            assert!(!spec.grants_relic_advancement);
        }

        assert_eq!(loot_spec_for_table("minecraft:chests/village/village_plains_house"), None);
    }

    #[test]
    fn loot_rolls_are_deterministic_and_balanced_across_10k_containers() {
        let spec = loot_spec_for_table(LOOT_PILLAGER_OUTPOST).unwrap();
        let mut ammo_hits = 0usize;
        let mut rifle_hits = 0usize;
        for i in 0..10_000_u64 {
            let seed = compute_container_seed(
                LOOT_PILLAGER_OUTPOST,
                DEFAULT_DIMENSION,
                [(i as f64) * 16.0, 72.0, -320.0],
                None,
                i.wrapping_mul(31),
            );
            let roll = roll_structure_loot(spec, seed);
            assert_eq!(roll, roll_structure_loot(spec, seed));
            if roll.cartridges > 0 {
                ammo_hits += 1;
                assert!((spec.rifle_bonus_ammo_min..=spec.ammo_max).contains(&roll.cartridges));
            }
            if roll.unloaded_rifle {
                rifle_hits += 1;
                // Finding a rare rifle in a watchtower always includes at least a handful of shells.
                assert!(roll.cartridges >= spec.rifle_bonus_ammo_min);
            }
        }
        // Expected ~10% rifles (~1,000) and ~47.8% total ammo (~4,780 including rifle bonus).
        assert!(
            (850..=1_150).contains(&rifle_hits),
            "unexpected outpost rifle frequency: {rifle_hits}/10000"
        );
        assert!(
            (4_400..=5_200).contains(&ammo_hits),
            "unexpected outpost ammo frequency: {ammo_hits}/10000"
        );
    }

    #[test]
    fn illager_and_hostile_monster_classifiers_distinguish_targets() {
        assert!(is_illager_kind("minecraft:pillager"));
        assert!(is_illager_kind("minecraft:vindicator"));
        assert!(is_illager_kind("minecraft:evoker"));
        assert!(is_illager_kind("minecraft:ravager"));
        assert!(!is_illager_kind("minecraft:skeleton"));
        assert!(!is_illager_kind("minecraft:villager"));

        assert!(is_hostile_monster_kind("minecraft:pillager"));
        assert!(is_hostile_monster_kind("minecraft:skeleton"));
        assert!(is_hostile_monster_kind("minecraft:creeper"));
        assert!(!is_hostile_monster_kind("minecraft:cow"));
        assert!(!is_hostile_monster_kind("minecraft:armor_stand"));
    }
}
