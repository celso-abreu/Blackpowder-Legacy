//! Server-authoritative `.44-40` firing, stance spread, muzzle VFX, submerged blackpowder
//! misfire & hydrodynamic bullet drag, per-shell side-gate reload loop, and `ItemPart` hooks.

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use mod_api::{
    CanGrindstoneRepairEvent, ClientHost, EntityRef, EntityUuid, EquipmentSlot, GameEventContext,
    Hand, Host, HostSide, IPayloadContext, Identifier, InteractionResult,
    IsCombineRepairableEvent, ItemBarEvent, ItemPart, ItemStack, LevelRef, LightLayer, LivingRef,
    LoopContext, OnUseTickEvent, ParticleKey, ParticleOptions, ReleaseUsingEvent, ServerHost,
    ShouldCauseReequipAnimationEvent, SoundSource, UseDurationEvent, UseItemEvent, VarInt,
    XpRepairRatioEvent,
};

use crate::ballistics::compute_voxel_ray_exit;
use crate::constants::*;
use crate::exploration::{distance_3d, grant_mod_criterion, record_bullet_shot};
use crate::skin::{default_skin_for_uuid, parse_mojang_skin_property};
use crate::{
    is_point_in_water, RifleAcousticSync, RifleInput, RifleSkinSync, RifleSync,
    RifleThirdPersonSync, AIMING, AIM_LAST_TICK, BUBBLE_KEY, BUBBLE_POP_KEY, CLIENT_RELOADING,
    CLIENT_SPRINTING, CLIENT_TICKS, FLAME_KEY, RIFLE_ACOUSTIC_SYNC, RIFLE_SKIN_SYNC, RIFLE_SYNC,
    RIFLE_TP_SYNC, SERVER_TICKS, SMOKE_KEY, SPLASH_KEY, WHITE_SMOKE_KEY,
};

static AIM_TICKS: Mutex<BTreeMap<u128, (i32, u64)>> = Mutex::new(BTreeMap::new());
static LEVER_READY: Mutex<BTreeMap<u128, u64>> = Mutex::new(BTreeMap::new());
static RELOAD_SESSIONS: Mutex<BTreeMap<u128, ReloadSession>> = Mutex::new(BTreeMap::new());
static ACTIVE_BULLETS: Mutex<Vec<ActiveBullet>> = Mutex::new(Vec::new());
static BARREL_HEAT: Mutex<BTreeMap<u128, BarrelHeatState>> = Mutex::new(BTreeMap::new());

#[derive(Clone, Copy)]
struct BarrelHeatState {
    heat: f32,
    last_shot_tick: u64,
    last_step_tick: u64,
    muzzle_mm: [i32; 3],
}

pub(crate) fn clear_barrel_heat() {
    BARREL_HEAT.lock().unwrap().clear();
}

pub(crate) fn add_shot_barrel_heat(current_heat: f32) -> f32 {
    (current_heat.max(0.0) + BARREL_HEAT_SHOT_GAIN).min(1.60)
}

pub(crate) fn step_barrel_heat(current_heat: f32, dt_ticks: u64) -> f32 {
    if !current_heat.is_finite() || current_heat <= 0.0 {
        return 0.0;
    }
    let cooled = current_heat - (dt_ticks.min(1_000) as f32) * BARREL_HEAT_COOL_PER_TICK;
    if cooled <= 1e-4 { 0.0 } else { cooled }
}

#[derive(Clone, Copy)]
struct ActiveBullet {
    entity: EntityRef,
    shooter_eid: u32,
    shooter_uuid: u128,
    origin: [f64; 3],
    prev_pos: [f64; 3],
    in_water: bool,
    terminated: bool,
    leaf_pierces: u8,
    whizzed_eids: [u32; 4],
    pending_splash: Option<[f64; 3]>,
    spawn_tick: u64,
    last_step_tick: u64,
}

impl ActiveBullet {
    fn has_whizzed(&self, eid: u32) -> bool {
        eid != 0 && self.whizzed_eids.contains(&eid)
    }

    fn mark_whizzed(&mut self, eid: u32) {
        if eid == 0 || self.has_whizzed(eid) {
            return;
        }
        for slot in &mut self.whizzed_eids {
            if *slot == 0 {
                *slot = eid;
                break;
            }
        }
    }
}

pub(crate) fn pack_coord_sub16(v: f64) -> VarInt {
    if !v.is_finite() {
        return VarInt(0);
    }
    let scaled = (v * 16.0).round().clamp(-500_000_000.0, 500_000_000.0) as i32;
    VarInt(scaled)
}

pub(crate) fn unpack_coord_sub16(v: VarInt) -> f64 {
    f64::from(v.0) * (1.0 / 16.0)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FlybyHit {
    pub(crate) closest_point: [f64; 3],
    pub(crate) is_supersonic_crack: bool,
    pub(crate) proximity: f32,
}

/// Evaluates whether a `.44-40` bullet segment `seg_a -> seg_b` reaches its true closest point of
/// approach past `listener_eye` (`<= FLYBY_MAX_RADIUS`), distinguishing sharp supersonic Mach
/// snaps (`<= 38m` & `<= 2.35m` miss) from transonic/subsonic Doppler lead whizzes downrange.
pub(crate) fn evaluate_bullet_flyby(
    muzzle_origin: [f64; 3],
    seg_a: [f64; 3],
    seg_b: [f64; 3],
    listener_eye: [f64; 3],
    terminated: bool,
) -> Option<FlybyHit> {
    let ab = [
        seg_b[0] - seg_a[0],
        seg_b[1] - seg_a[1],
        seg_b[2] - seg_a[2],
    ];
    let len_sq = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
    if !len_sq.is_finite() || len_sq < 1e-6 {
        return None;
    }

    let b_to_eye = [
        listener_eye[0] - seg_b[0],
        listener_eye[1] - seg_b[1],
        listener_eye[2] - seg_b[2],
    ];
    let ahead_dot = b_to_eye[0] * ab[0] + b_to_eye[1] * ab[1] + b_to_eye[2] * ab[2];

    // While still in flight, defer until `seg_b` reaches or crosses the listener's perpendicular
    // plane so we always capture the true global minimum distance rather than an early approach step.
    if !terminated && ahead_dot > 0.0 {
        return None;
    }

    let a_to_eye = [
        listener_eye[0] - seg_a[0],
        listener_eye[1] - seg_a[1],
        listener_eye[2] - seg_a[2],
    ];
    let t = ((a_to_eye[0] * ab[0] + a_to_eye[1] * ab[1] + a_to_eye[2] * ab[2]) / len_sq)
        .clamp(0.0, 1.0);
    let closest = [
        seg_a[0] + ab[0] * t,
        seg_a[1] + ab[1] * t,
        seg_a[2] + ab[2] * t,
    ];
    let miss_distance = distance_3d(listener_eye, closest);
    if !miss_distance.is_finite() || miss_distance > FLYBY_MAX_RADIUS {
        return None;
    }

    // If the bullet terminated BEFORE reaching the listener's plane (e.g. hitting the ground in
    // front of them), only trigger a grazing whiz when the impact itself is tight (`<= 2.75m`).
    if terminated && ahead_dot > 0.0 && miss_distance > 2.75 {
        return None;
    }

    let travel_distance = distance_3d(muzzle_origin, closest);
    if !travel_distance.is_finite() || travel_distance < FLYBY_MIN_MUZZLE_DIST {
        return None;
    }

    let is_supersonic_crack =
        miss_distance <= FLYBY_CRACK_RADIUS && travel_distance <= FLYBY_SUPERSONIC_RANGE;
    let proximity = (1.0 - (miss_distance / FLYBY_MAX_RADIUS)).clamp(0.0, 1.0) as f32;
    Some(FlybyHit {
        closest_point: closest,
        is_supersonic_crack,
        proximity,
    })
}

/// Pure acoustic classifier mapping sky visibility, sky-light level, overhead ceiling probes, and
/// horizontal wall reflections around the muzzle into `(ACOUSTIC_CUE_SHOT_*, intensity_pm)`.
pub(crate) fn classify_shot_acoustics(
    can_see_sky: bool,
    sky_light: i32,
    low_ceiling: bool,
    med_ceiling: bool,
    tight_walls: u8,
    reflect_walls: u8,
) -> (i32, i32) {
    let sky = sky_light.clamp(0, 15);
    let tw = i32::from(tight_walls.min(4));
    let rw = i32::from(reflect_walls.min(4));

    if !can_see_sky
        && (med_ceiling || sky <= 3)
        && (sky <= 4 || tw >= 2 || (low_ceiling && tw >= 1))
    {
        let intensity =
            (480 + tw * 105 + if low_ceiling { 120 } else { 0 } + (4 - sky.min(4)) * 30)
                .clamp(480, 1000);
        (ACOUSTIC_CUE_SHOT_CONFINED, intensity)
    } else if !can_see_sky && (med_ceiling || low_ceiling) {
        let intensity = (420 + tw * 90 + rw * 55).clamp(420, 880);
        (ACOUSTIC_CUE_SHOT_SEMI, intensity)
    } else {
        let intensity = (380 + rw * 145 + tw * 35).clamp(380, 1000);
        (ACOUSTIC_CUE_SHOT_OPEN, intensity)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EntityImpactKind {
    ShieldBlock,
    ArmorMetal,
    RigidConstruct,
    Flesh,
}

fn is_metallic_armor_id(item_id: &str) -> bool {
    let path = item_id.split_once(':').map_or(item_id, |(_, p)| p);
    if path.starts_with("leather_") || path.starts_with("turtle_") || path == "wolf_armor" {
        return false;
    }
    path.starts_with("iron_")
        || path.starts_with("chainmail_")
        || path.starts_with("golden_")
        || path.starts_with("diamond_")
        || path.starts_with("netherite_")
        || path.ends_with("_horse_armor")
}

/// Classifies the ballistic target material when a `.44-40` slug hits an entity so armored targets,
/// shields, golems, skeletons, and unarmored bodies produce distinct 3D impact audio.
pub(crate) fn classify_entity_impact_kind(
    entity_kind_id: &str,
    armor_item_id: Option<&str>,
    blocking_front: bool,
) -> EntityImpactKind {
    if blocking_front {
        return EntityImpactKind::ShieldBlock;
    }
    if armor_item_id.is_some_and(is_metallic_armor_id) {
        return EntityImpactKind::ArmorMetal;
    }
    let path = entity_kind_id
        .split_once(':')
        .map_or(entity_kind_id, |(_, p)| p);
    if matches!(
        path,
        "iron_golem" | "copper_golem" | "shulker" | "blaze"
    ) || path.contains("minecart")
    {
        return EntityImpactKind::ArmorMetal;
    }
    if matches!(
        path,
        "skeleton"
            | "stray"
            | "bogged"
            | "wither_skeleton"
            | "skeleton_horse"
            | "wither"
            | "armor_stand"
            | "creaking"
    ) || path.contains("boat")
        || path.contains("raft")
    {
        return EntityImpactKind::RigidConstruct;
    }
    EntityImpactKind::Flesh
}

pub(crate) fn emit_entity_ballistic_impact_sound(
    host: &mut dyn ServerHost,
    level: LevelRef,
    victim_id: u32,
    hit_at: [f64; 3],
    ray_dir: [f64; 3],
    aabb: Option<[[f64; 3]; 2]>,
    is_headshot: bool,
    now: u64,
) -> EntityImpactKind {
    let Some(victim_ent) = host.entity(victim_id) else {
        return EntityImpactKind::Flesh;
    };
    let kind_id = victim_ent
        .kind(host)
        .map(|k| k.id(host).to_string())
        .unwrap_or_default();

    let mut blocking_front = false;
    let mut armor_id: Option<String> = None;

    if let Some(liv) = host.living(victim_ent) {
        if liv.is_blocking(host).unwrap_or(false)
            && let Some(look) = victim_ent.view_vector(host)
        {
            let horiz_dot = look[0] * ray_dir[0] + look[2] * ray_dir[2];
            blocking_front = horiz_dot < -0.05;
        }

        let hit_slot = if is_headshot {
            EquipmentSlot::Head
        } else if let Some(b) = aabb {
            let h = (b[1][1] - b[0][1]).max(0.1);
            let rel_y = ((hit_at[1] - b[0][1]) / h).clamp(0.0, 1.0);
            if rel_y >= 0.52 {
                EquipmentSlot::Chest
            } else if rel_y >= 0.24 {
                EquipmentSlot::Legs
            } else {
                EquipmentSlot::Feet
            }
        } else {
            EquipmentSlot::Chest
        };
        armor_id = liv
            .equipment(host, hit_slot)
            .map(|st| st.item().id(host).to_string());
    }

    let impact_kind = classify_entity_impact_kind(&kind_id, armor_id.as_deref(), blocking_front);
    let j = shot_pitch_jitter(now, u128::from(victim_id) * 0x9E37_79B9);
    let underwater = is_point_in_water(host, level, hit_at);
    let water_vol = if underwater { 0.72 } else { 1.0 };
    let water_pitch = if underwater { 0.78 } else { 1.0 };

    let (snd, base_vol, base_pitch) = match impact_kind {
        EntityImpactKind::ShieldBlock => (SND_IMPACT_ARMOR, 0.96, 0.86 + 0.03 * j),
        EntityImpactKind::ArmorMetal => (
            SND_IMPACT_ARMOR,
            if is_headshot { 1.00 } else { 0.92 },
            (if is_headshot { 1.12 } else { 0.98 }) + 0.04 * j,
        ),
        EntityImpactKind::RigidConstruct => (SND_IMPACT_FLESH, 0.86, 1.25 + 0.04 * j),
        EntityImpactKind::Flesh => (
            SND_IMPACT_FLESH,
            if is_headshot { 0.96 } else { 0.86 },
            (if is_headshot { 1.10 } else { 0.99 }) + 0.04 * j,
        ),
    };
    level.play_sound(
        host,
        snd,
        hit_at,
        SoundSource::Players,
        base_vol * water_vol,
        (base_pitch * water_pitch).clamp(0.5, 2.0),
    );
    impact_kind
}

#[derive(Clone, Copy)]
enum ReloadPhase {
    Opening { until: u64 },
    Inserting { next_tick: u64 },
    Closing { until: u64, empty_chamber: bool },
}

#[derive(Clone, Copy)]
struct ReloadSession {
    slot: u32,
    from_empty: bool,
    inserted_any: bool,
    phase: ReloadPhase,
}

pub(crate) struct Winchester;

pub(crate) fn rounds(stack: &ItemStack) -> u32 {
    stack.damage().min(TUBE_CAPACITY)
}

fn shooter(host: &dyn ServerHost) -> Option<mod_api::PlayerRef> {
    host.players().acting_player()
}

fn is_player_reloading(uuid: u128) -> bool {
    RELOAD_SESSIONS.lock().unwrap().contains_key(&uuid)
}

fn cancel_reload_session(host: &mut dyn ServerHost, player: mod_api::PlayerRef) -> bool {
    let removed = RELOAD_SESSIONS
        .lock()
        .unwrap()
        .remove(&player.uuid())
        .is_some();
    if removed {
        let cur = held_rifle(host, player).map_or(0, |(_, r)| rounds(&r));
        send_sync(host, player, SYNC_RELOAD_ABORTED, cur);
        broadcast_tp_state(host, player, TP_STATE_IDLE);
    }
    removed
}

pub(crate) fn on_rifle_input(
    value: RifleInput,
    cx: &mut dyn IPayloadContext,
) -> Result<(), String> {
    let player = cx.player().map_err(|e| format!("{e:?}"))?;
    let host = cx.server_host().map_err(|e| format!("{e:?}"))?;
    match value.action.0 {
        ACTION_FIRE => {
            if let Some(smoke) = SMOKE_KEY.get().copied() {
                fire_rifle(
                    host,
                    player,
                    smoke,
                    [
                        value.muzzle_right_mm.0,
                        value.muzzle_up_mm.0,
                        value.muzzle_fwd_mm.0,
                    ],
                    value.aim_pct.0,
                );
            }
        }
        ACTION_RELOAD => start_reload(host, player),
        ACTION_RELOAD_CANCEL => {
            let _ = cancel_reload_session(host, player);
        }
        ACTION_SKIN_REQUEST => sync_player_skin(host, player),
        _ => {}
    }
    Ok(())
}

fn send_sync(host: &mut dyn ServerHost, player: mod_api::PlayerRef, kind: i32, rounds_now: u32) {
    if let Some(sync) = RIFLE_SYNC.get() {
        let _ = sync.send(
            host,
            player,
            &RifleSync {
                kind: VarInt(kind),
                rounds: VarInt(rounds_now as i32),
            },
        );
    }
}

fn broadcast_tp_state(host: &mut dyn ServerHost, actor: mod_api::PlayerRef, state: i32) {
    let Some(chan) = RIFLE_TP_SYNC.get() else {
        return;
    };
    let actor_ent = actor.entity();
    let Some(level) = actor_ent.level(host) else {
        return;
    };
    let Some(actor_pos) = actor_ent.position(host) else {
        return;
    };
    let Some(players) = level.online_players(host) else {
        return;
    };
    let payload = RifleThirdPersonSync {
        entity_id: VarInt(actor_ent.id() as i32),
        state: VarInt(state),
    };
    for p in players {
        if p == actor {
            continue;
        }
        if let Some(ppos) = p.entity().position(host) {
            let dx = ppos[0] - actor_pos[0];
            let dy = ppos[1] - actor_pos[1];
            let dz = ppos[2] - actor_pos[2];
            if dx * dx + dy * dy + dz * dz <= TP_BROADCAST_RADIUS_SQ {
                let _ = chan.send(host, p, &payload);
            }
        }
    }
}

/// Returns the sub-tick `(ray_origin, ray_dir)` of the tracked in-flight `.44-40` bullet nearest
/// to `hit_at`, enabling exact cranial ray-AABB intersection even on long curved trajectories.
pub(crate) fn find_active_bullet_ray(
    host: &dyn Host,
    hit_at: [f64; 3],
) -> Option<([f64; 3], [f64; 3])> {
    let active = ACTIVE_BULLETS.lock().unwrap();
    let mut best: Option<(f64, [f64; 3], [f64; 3])> = None;
    for b in active.iter() {
        let Some(vel) = b.entity.velocity(host) else {
            continue;
        };
        let end = [
            b.prev_pos[0] + vel[0],
            b.prev_pos[1] + vel[1],
            b.prev_pos[2] + vel[2],
        ];
        let dx = end[0] - hit_at[0];
        let dy = end[1] - hit_at[1];
        let dz = end[2] - hit_at[2];
        let d2 = dx * dx + dy * dy + dz * dz;
        if d2 <= 900.0 && best.is_none_or(|(best_d2, _, _)| d2 < best_d2) {
            best = Some((d2, b.prev_pos, vel));
        }
    }
    best.map(|(_, origin, dir)| (origin, dir))
}

fn sync_player_skin(host: &mut dyn ServerHost, player: mod_api::PlayerRef) {
    let Some(skin_chan) = RIFLE_SKIN_SYNC.get() else {
        return;
    };
    let uuid = player.uuid();
    let (default_sprite, default_slim) = default_skin_for_uuid(uuid);

    let profile = host
        .profile_by_uuid(uuid)
        .or_else(|| player.profile(host));

    let (sprite, slim) = profile
        .as_ref()
        .and_then(|prof| {
            prof.properties
                .iter()
                .find(|prop| prop.name == "textures")
                .and_then(|prop| parse_mojang_skin_property(&prop.value))
        })
        .unwrap_or_else(|| (default_sprite.to_string(), default_slim));

    let _ = skin_chan.send(
        host,
        player,
        &RifleSkinSync {
            sprite,
            fallback: default_sprite.to_string(),
            slim: VarInt(i32::from(slim)),
        },
    );
}

fn play_world_sound(
    host: &mut dyn ServerHost,
    player: mod_api::PlayerRef,
    sound: &str,
    volume: f32,
    pitch: f32,
) {
    let entity = player.entity();
    let Some(level) = entity.level(host) else {
        return;
    };
    let Some(at) = entity.position(host) else {
        return;
    };
    level.play_sound(host, sound, at, SoundSource::Players, volume, pitch);
}

#[derive(Clone, Copy)]
struct ShooterFrame {
    level: LevelRef,
    eye: [f64; 3],
    breech: [f64; 3],
    muzzle: [f64; 3],
    forward: [f64; 3],
    right: [f64; 3],
    up: [f64; 3],
}

fn compute_shooter_frame(
    host: &dyn ServerHost,
    player: mod_api::PlayerRef,
    muzzle_mm: [i32; 3],
) -> Option<ShooterFrame> {
    let entity = player.entity();
    let level = entity.level(host)?;
    let eye = entity.eye_position(host)?;
    let forward = entity.view_vector(host)?;
    let up = entity.up_vector(host).unwrap_or([0.0, 1.0, 0.0]);
    // Exact camera-right unit vector (`forward × up`), singularity-free at every pitch/yaw.
    let right = [
        forward[1] * up[2] - forward[2] * up[1],
        forward[2] * up[0] - forward[0] * up[2],
        forward[0] * up[1] - forward[1] * up[0],
    ];

    let has_client_offset = muzzle_mm[2] > 0;
    let right_m = if has_client_offset {
        (f64::from(muzzle_mm[0]) / 1000.0).clamp(-1.20, 1.20)
    } else {
        0.34
    };
    let up_m = if has_client_offset {
        (f64::from(muzzle_mm[1]) / 1000.0).clamp(-1.20, 1.20)
    } else {
        -0.22
    };
    let fwd_m = if has_client_offset {
        (f64::from(muzzle_mm[2]) / 1000.0).clamp(0.80, 2.20)
    } else {
        1.35
    };

    let breech = [
        eye[0] + right[0] * (right_m * 0.45) + up[0] * (up_m * 0.55) + forward[0] * 0.28,
        eye[1] + right[1] * (right_m * 0.45) + up[1] * (up_m * 0.55) + forward[1] * 0.28,
        eye[2] + right[2] * (right_m * 0.45) + up[2] * (up_m * 0.55) + forward[2] * 0.28,
    ];
    let muzzle = [
        eye[0] + right[0] * right_m + up[0] * up_m + forward[0] * fwd_m,
        eye[1] + right[1] * right_m + up[1] * up_m + forward[1] * fwd_m,
        eye[2] + right[2] * right_m + up[2] * up_m + forward[2] * fwd_m,
    ];

    Some(ShooterFrame {
        level,
        eye,
        breech,
        muzzle,
        forward,
        right,
        up,
    })
}

fn is_eye_or_breech_submerged(
    host: &dyn ServerHost,
    player: mod_api::PlayerRef,
    frame: &ShooterFrame,
) -> bool {
    if matches!(
        player.entity().eye_fluid(host),
        Some(Some(ref f)) if f == WATER_FLUID || f == FLOWING_WATER_FLUID
    ) {
        return true;
    }
    is_point_in_water(host, frame.level, frame.eye)
        || is_point_in_water(host, frame.level, frame.breech)
}

fn is_player_submerged_simple(host: &dyn ServerHost, player: mod_api::PlayerRef) -> bool {
    let entity = player.entity();
    if matches!(
        entity.eye_fluid(host),
        Some(Some(ref f)) if f == WATER_FLUID || f == FLOWING_WATER_FLUID
    ) {
        return true;
    }
    let Some(level) = entity.level(host) else {
        return false;
    };
    let Some(eye) = entity.eye_position(host) else {
        return false;
    };
    is_point_in_water(host, level, eye)
}

fn shot_pitch_jitter(now: u64, uuid: u128) -> f32 {
    let mut h = now
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add((uuid as u64) ^ ((uuid >> 64) as u64));
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    (((h & 0xFFFF) as f32) * (1.0 / 32_767.5) - 1.0).clamp(-1.0, 1.0)
}

fn emit_wet_misfire(
    host: &mut dyn ServerHost,
    player: mod_api::PlayerRef,
    frame: &ShooterFrame,
    loaded: u32,
) {
    let j = shot_pitch_jitter(SERVER_TICKS.load(Ordering::Relaxed), player.uuid());
    frame.level.play_sound(
        host,
        SND_MISFIRE_WET,
        frame.breech,
        SoundSource::Players,
        0.86,
        (0.98 + 0.04 * j).clamp(0.85, 1.12),
    );

    if let Some(bubble) = BUBBLE_KEY.get().copied() {
        let bubble_opts = ParticleOptions::simple(bubble);
        frame.level.send_particles(
            host,
            &bubble_opts,
            None,
            frame.breech,
            [0.04, 0.04, 0.04],
            0.03,
            5,
        );
        if is_point_in_water(host, frame.level, frame.muzzle) {
            frame.level.send_particles(
                host,
                &bubble_opts,
                None,
                frame.muzzle,
                [0.03, 0.03, 0.03],
                0.04,
                4,
            );
        }
    }
    if let Some(pop) = BUBBLE_POP_KEY.get().copied() {
        let pop_opts = ParticleOptions::simple(pop);
        frame.level.send_particles(
            host,
            &pop_opts,
            None,
            frame.breech,
            [0.03, 0.03, 0.03],
            0.015,
            3,
        );
    }

    send_sync(host, player, SYNC_WET_MISFIRE, loaded);
}

fn muzzle_effects(
    host: &mut dyn ServerHost,
    frame: &ShooterFrame,
    smoke: ParticleKey,
    aim_pct: i32,
    muzzle_in_water: bool,
) {
    let level = frame.level;
    let muzzle = frame.muzzle;
    let forward = frame.forward;
    let right = frame.right;
    let up = frame.up;
    let aimed = aim_pct >= 65;

    // When the shooter's head & receiver are dry above water but `boca_cano` dips below the
    // water surface, blackpowder gases vent straight into the water as a pressurized cavitation
    // bubble + surface-spray burst instead of dry flame and smoke.
    if muzzle_in_water {
        if let Some(bubble) = BUBBLE_KEY.get().copied() {
            let bubble_opts = ParticleOptions::simple(bubble);
            level.send_particles(
                host,
                &bubble_opts,
                None,
                muzzle,
                [0.08, 0.08, 0.08],
                0.18,
                12,
            );
        }
        if let Some(splash) = SPLASH_KEY.get().copied() {
            let splash_opts = ParticleOptions::simple(splash);
            level.send_particles(
                host,
                &splash_opts,
                None,
                [muzzle[0], muzzle[1] + 0.08, muzzle[2]],
                [0.10, 0.05, 0.10],
                0.16,
                10,
            );
        }
        if let Some(pop) = BUBBLE_POP_KEY.get().copied() {
            let pop_opts = ParticleOptions::simple(pop);
            level.send_particles(
                host,
                &pop_opts,
                None,
                muzzle,
                [0.06, 0.06, 0.06],
                0.04,
                5,
            );
        }
        return;
    }

    // 1. Tight muzzle flash spark right on the barrel crown (`boca_cano`).
    if let Some(flame) = FLAME_KEY.get().copied() {
        let flame_opts = ParticleOptions::simple(flame);
        level.send_particles(
            host,
            &flame_opts,
            None,
            muzzle,
            [0.012, 0.012, 0.012],
            0.006,
            if aimed { 1 } else { 2 },
        );
    }

    // 2. Directional pale-grey blackpowder smoke (`white_smoke`) jetting forward along the bore
    //    (`count = 0` sends one directional particle moving along `spread * speed`).
    if let Some(white_smoke) = WHITE_SMOKE_KEY.get().copied() {
        let ws_opts = ParticleOptions::simple(white_smoke);
        let speeds: &[f64] = if aimed {
            &[0.14, 0.22]
        } else {
            &[0.08, 0.14, 0.20, 0.26]
        };
        let origin = if aimed {
            [
                muzzle[0] - up[0] * 0.025 + forward[0] * 0.06,
                muzzle[1] - up[1] * 0.025 + forward[1] * 0.06,
                muzzle[2] - up[2] * 0.025 + forward[2] * 0.06,
            ]
        } else {
            muzzle
        };
        for (idx, &spd) in speeds.iter().enumerate() {
            let side_jitter = ((idx as f64) - 1.5) * 0.035;
            let dir = [
                forward[0] + right[0] * side_jitter + up[0] * 0.06,
                forward[1] + right[1] * side_jitter + up[1] * 0.06,
                forward[2] + right[2] * side_jitter + up[2] * 0.06,
            ];
            level.send_particles(host, &ws_opts, None, origin, dir, spd, 0);
        }
    }

    // 3. Subtle lingering powder wisp just past the crown.
    let wisp_pos = [
        muzzle[0] + forward[0] * 0.14,
        muzzle[1] + forward[1] * 0.14,
        muzzle[2] + forward[2] * 0.14,
    ];
    let smoke_opts = ParticleOptions::simple(smoke);
    level.send_particles(
        host,
        &smoke_opts,
        None,
        wisp_pos,
        [0.022, 0.022, 0.022],
        0.008,
        if aimed { 1 } else { 3 },
    );
}

/// Computes total travel distance (`blocks`) for a bullet of speed `speed` (`blocks/tick`) that
/// crosses from air into water at `surface_dist`: full speed in air up to the surface, then
/// severe hydrodynamic deceleration capping first-tick water penetration to `<= 2.10` blocks.
pub(crate) fn compute_water_entry_step(speed: f64, surface_dist: f64) -> f64 {
    let air_dist = surface_dist.clamp(0.0, speed.max(0.0));
    let rem = (speed - air_dist).max(0.0);
    air_dist + (rem * 0.16).min(WATER_FIRST_TICK_PENETRATION)
}

/// Advances an underwater `.44-40` soft-lead bullet by one server tick under strong hydrodynamic
/// drag and gentle lead sink; returns `None` once speed drops below lethal threshold.
pub(crate) fn step_underwater_bullet_velocity(vel: [f64; 3]) -> Option<[f64; 3]> {
    let next = [
        vel[0] * WATER_BULLET_DRAG,
        vel[1] * WATER_BULLET_DRAG - 0.03,
        vel[2] * WATER_BULLET_DRAG,
    ];
    let speed_sq = next[0] * next[0] + next[1] * next[1] + next[2] * next[2];
    if !speed_sq.is_finite() || speed_sq < WATER_BULLET_STOP_SPEED * WATER_BULLET_STOP_SPEED {
        None
    } else {
        Some(next)
    }
}

fn point_along(origin: [f64; 3], dir: [f64; 3], dist: f64) -> [f64; 3] {
    [
        origin[0] + dir[0] * dist,
        origin[1] + dir[1] * dist,
        origin[2] + dir[2] * dist,
    ]
}

fn find_water_surface_along_ray(
    host: &dyn ServerHost,
    level: LevelRef,
    origin: [f64; 3],
    dir: [f64; 3],
    max_dist: f64,
) -> Option<(f64, [f64; 3])> {
    if max_dist <= 0.05 {
        return None;
    }
    let step = 0.45_f64;
    let mut s = 0.30_f64.min(max_dist);
    let mut hit_dist = None;
    while s <= max_dist {
        if is_point_in_water(host, level, point_along(origin, dir, s)) {
            hit_dist = Some(s);
            break;
        }
        if (max_dist - s).abs() < 1e-6 {
            break;
        }
        s = (s + step).min(max_dist);
    }

    let mut hi = hit_dist?;
    let mut lo = (hi - step).max(0.0);
    for _ in 0..4 {
        let mid = 0.5 * (lo + hi);
        if is_point_in_water(host, level, point_along(origin, dir, mid)) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    Some((hi, point_along(origin, dir, hi)))
}

fn emit_water_entry_splash(host: &mut dyn ServerHost, level: LevelRef, at: [f64; 3]) {
    level.play_sound(host, SND_WATER_SPLASH, at, SoundSource::Players, 0.78, 1.42);
    if let Some(splash) = SPLASH_KEY.get().copied() {
        let splash_opts = ParticleOptions::simple(splash);
        level.send_particles(
            host,
            &splash_opts,
            None,
            [at[0], at[1] + 0.05, at[2]],
            [0.09, 0.04, 0.09],
            0.16,
            10,
        );
    }
    if let Some(pop) = BUBBLE_POP_KEY.get().copied() {
        let pop_opts = ParticleOptions::simple(pop);
        level.send_particles(
            host,
            &pop_opts,
            None,
            [at[0], at[1] + 0.02, at[2]],
            [0.06, 0.03, 0.06],
            0.03,
            5,
        );
    }
}

fn emit_cavitation_trail(
    host: &mut dyn ServerHost,
    level: LevelRef,
    from: [f64; 3],
    to: [f64; 3],
) {
    let Some(bubble) = BUBBLE_KEY.get().copied() else {
        return;
    };
    let dx = to[0] - from[0];
    let dy = to[1] - from[1];
    let dz = to[2] - from[2];
    let dist = (dx * dx + dy * dy + dz * dz).sqrt();
    if !dist.is_finite() || dist < 0.04 {
        return;
    }
    let samples = ((dist / 0.42).ceil() as usize).clamp(1, 6);
    let bubble_opts = ParticleOptions::simple(bubble);
    for i in 1..=samples {
        let t = (i as f64) / (samples as f64);
        let p = [from[0] + dx * t, from[1] + dy * t, from[2] + dz * t];
        if is_point_in_water(host, level, p) {
            level.send_particles(
                host,
                &bubble_opts,
                None,
                p,
                [0.025, 0.025, 0.025],
                0.015,
                2,
            );
        }
    }
}

fn probe_and_broadcast_shot_acoustics(
    host: &mut dyn ServerHost,
    shooter: mod_api::PlayerRef,
    muzzle: [f64; 3],
    level: LevelRef,
) {
    let Some(chan) = RIFLE_ACOUSTIC_SYNC.get() else {
        return;
    };
    let bp = [
        muzzle[0].floor() as i32,
        muzzle[1].floor() as i32,
        muzzle[2].floor() as i32,
    ];
    let can_see_sky = level.can_see_sky(host, bp).unwrap_or(false);
    let sky_light = level
        .brightness(host, LightLayer::Sky, bp)
        .unwrap_or(if can_see_sky { 15 } else { 0 })
        .clamp(0, 15);
    let low_ceiling = !level
        .line_of_sight(host, muzzle, [muzzle[0], muzzle[1] + 5.0, muzzle[2]])
        .unwrap_or(true);
    let med_ceiling = low_ceiling
        || !level
            .line_of_sight(host, muzzle, [muzzle[0], muzzle[1] + 14.0, muzzle[2]])
            .unwrap_or(true);

    let mut tight_walls = 0_u8;
    let mut reflect_walls = 0_u8;
    for (dx, dz) in [(1.0_f64, 0.0_f64), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
        let hit_tight = !level
            .line_of_sight(
                host,
                muzzle,
                [muzzle[0] + dx * 6.5, muzzle[1] + 0.2, muzzle[2] + dz * 6.5],
            )
            .unwrap_or(true);
        if hit_tight {
            tight_walls += 1;
            reflect_walls += 1;
        } else {
            let hit_far = !level
                .line_of_sight(
                    host,
                    muzzle,
                    [muzzle[0] + dx * 24.0, muzzle[1] + 1.2, muzzle[2] + dz * 24.0],
                )
                .unwrap_or(true);
            if hit_far {
                reflect_walls += 1;
            }
        }
    }

    let (cue, intensity_pm) = classify_shot_acoustics(
        can_see_sky,
        sky_light,
        low_ceiling,
        med_ceiling,
        tight_walls,
        reflect_walls,
    );
    let payload = RifleAcousticSync {
        cue: VarInt(cue),
        x_sub16: pack_coord_sub16(muzzle[0]),
        y_sub16: pack_coord_sub16(muzzle[1]),
        z_sub16: pack_coord_sub16(muzzle[2]),
        intensity_pm: VarInt(intensity_pm),
    };

    if let Some(players) = level.online_players(host) {
        for p in players {
            let near = p == shooter
                || p.entity().position(host).is_some_and(|pos| {
                    let dx = pos[0] - muzzle[0];
                    let dy = pos[1] - muzzle[1];
                    let dz = pos[2] - muzzle[2];
                    dx * dx + dy * dy + dz * dz <= ACOUSTIC_BROADCAST_RADIUS_SQ
                });
            if near {
                let _ = chan.send(host, p, &payload);
            }
        }
    }
}

fn emit_segment_flybys(
    host: &mut dyn ServerHost,
    level: LevelRef,
    bullet: &mut ActiveBullet,
    seg_a: [f64; 3],
    seg_b: [f64; 3],
    skip_victim_eid: Option<u32>,
    terminated: bool,
) {
    let Some(chan) = RIFLE_ACOUSTIC_SYNC.get() else {
        return;
    };
    let Some(players) = level.online_players(host) else {
        return;
    };
    for p in players {
        let pid = p.entity().id();
        if pid == 0
            || pid == bullet.shooter_eid
            || Some(pid) == skip_victim_eid
            || bullet.has_whizzed(pid)
        {
            continue;
        }
        let Some(eye) = p.entity().eye_position(host) else {
            continue;
        };
        let Some(hit) = evaluate_bullet_flyby(bullet.origin, seg_a, seg_b, eye, terminated) else {
            continue;
        };
        if is_point_in_water(host, level, eye) {
            continue;
        }
        if !level
            .line_of_sight(host, hit.closest_point, eye)
            .unwrap_or(true)
        {
            continue;
        }
        bullet.mark_whizzed(pid);
        let payload = RifleAcousticSync {
            cue: VarInt(if hit.is_supersonic_crack {
                ACOUSTIC_CUE_BULLET_CRACK
            } else {
                ACOUSTIC_CUE_BULLET_WHIZ
            }),
            x_sub16: pack_coord_sub16(hit.closest_point[0]),
            y_sub16: pack_coord_sub16(hit.closest_point[1]),
            z_sub16: pack_coord_sub16(hit.closest_point[2]),
            intensity_pm: VarInt((hit.proximity * 1000.0).round().clamp(0.0, 1000.0) as i32),
        };
        let _ = chan.send(host, p, &payload);
    }
}

/// Flushes terminal-tick bullet flybys when a `.44-40` projectile impacts terrain or an entity
/// mid-tick (`projectile_impact` runs before `level_tick_post` and discards the projectile).
pub(crate) fn flush_bullet_flyby_on_impact(
    host: &mut dyn ServerHost,
    level: LevelRef,
    owner_eid: u32,
    hit_at: [f64; 3],
    skip_victim_eid: Option<u32>,
) {
    let matched_idx = {
        let active = ACTIVE_BULLETS.lock().unwrap();
        let mut best: Option<(usize, f64)> = None;
        for (idx, b) in active.iter().enumerate() {
            if b.terminated || b.in_water {
                continue;
            }
            if owner_eid != 0 && b.shooter_eid != 0 && b.shooter_eid != owner_eid {
                continue;
            }
            let end = if let Some(vel) = b.entity.velocity(host) {
                [
                    b.prev_pos[0] + vel[0],
                    b.prev_pos[1] + vel[1],
                    b.prev_pos[2] + vel[2],
                ]
            } else {
                b.prev_pos
            };
            let dx = end[0] - hit_at[0];
            let dy = end[1] - hit_at[1];
            let dz = end[2] - hit_at[2];
            let d2 = dx * dx + dy * dy + dz * dz;
            if d2 <= 900.0 && best.is_none_or(|(_, best_d2)| d2 < best_d2) {
                best = Some((idx, d2));
            }
        }
        best.map(|(i, _)| i)
    };

    let Some(idx) = matched_idx else {
        return;
    };
    let mut bullet_copy = {
        let mut active = ACTIVE_BULLETS.lock().unwrap();
        let Some(b) = active.get_mut(idx) else {
            return;
        };
        b.terminated = true;
        *b
    };
    let seg_a = bullet_copy.prev_pos;
    emit_segment_flybys(
        host,
        level,
        &mut bullet_copy,
        seg_a,
        hit_at,
        skip_victim_eid,
        true,
    );
    if let Some(b) = ACTIVE_BULLETS.lock().unwrap().get_mut(idx) {
        b.whizzed_eids = bullet_copy.whizzed_eids;
    }
}

fn register_spawned_bullet(
    host: &mut dyn ServerHost,
    entity: EntityRef,
    shooter_eid: u32,
    shooter_uuid: u128,
    origin: [f64; 3],
    muzzle_in_water: bool,
    leaf_pierces: u8,
    inherited_whizzed: [u32; 4],
    now: u64,
) {
    let Some(level) = entity.level(host) else {
        return;
    };
    let Some(pos0) = entity.position(host) else {
        return;
    };
    let Some(vel0) = entity.velocity(host) else {
        return;
    };
    let speed = (vel0[0] * vel0[0] + vel0[1] * vel0[1] + vel0[2] * vel0[2]).sqrt();
    if !speed.is_finite() || speed < 1e-4 {
        return;
    }
    let dir = [vel0[0] / speed, vel0[1] / speed, vel0[2] / speed];

    let starts_in_water = muzzle_in_water || is_point_in_water(host, level, pos0);
    let mut pending_splash = None;

    if starts_in_water {
        let capped = speed.min(WATER_FIRST_TICK_PENETRATION);
        let _ = entity.set_velocity(host, [dir[0] * capped, dir[1] * capped, dir[2] * capped]);
    } else if let Some((s_surf, entry_pos)) =
        find_water_surface_along_ray(host, level, pos0, dir, speed)
    {
        let step_dist = compute_water_entry_step(speed, s_surf);
        let _ = entity.set_velocity(
            host,
            [dir[0] * step_dist, dir[1] * step_dist, dir[2] * step_dist],
        );
        pending_splash = Some(entry_pos);
    }

    let mut active = ACTIVE_BULLETS.lock().unwrap();
    if active.len() >= 64 {
        active.remove(0);
    }
    active.push(ActiveBullet {
        entity,
        shooter_eid,
        shooter_uuid,
        origin,
        prev_pos: pos0,
        in_water: starts_in_water,
        terminated: false,
        leaf_pierces,
        whizzed_eids: inherited_whizzed,
        pending_splash,
        spawn_tick: now,
        last_step_tick: now,
    });
}

/// Perforates up to `MAX_LEAF_PERFORATIONS` (`2`) dry `#minecraft:leaves` blocks per shot by
/// spawning a continuation `.44-40` projectile `2 mm` past the far voxel face with `72%` retained
/// velocity, preserving original muzzle distance, headshot ray geometry, and flyby deduplication.
pub(crate) fn try_perforate_foliage_on_impact(
    host: &mut dyn ServerHost,
    _level: LevelRef,
    owner_eid: u32,
    block: [i32; 3],
    hit_at: [f64; 3],
) -> bool {
    if owner_eid == 0 {
        return false;
    }
    let now = SERVER_TICKS.load(Ordering::Relaxed);
    let candidate = {
        let active = ACTIVE_BULLETS.lock().unwrap();
        let mut best: Option<(usize, f64, ActiveBullet)> = None;
        for (i, b) in active.iter().enumerate() {
            if b.shooter_eid != owner_eid
                || b.in_water
                || b.leaf_pierces >= MAX_LEAF_PERFORATIONS
            {
                continue;
            }
            let vel = b.entity.velocity(host).unwrap_or([0.0; 3]);
            let end = [
                b.prev_pos[0] + vel[0],
                b.prev_pos[1] + vel[1],
                b.prev_pos[2] + vel[2],
            ];
            let dx = end[0] - hit_at[0];
            let dy = end[1] - hit_at[1];
            let dz = end[2] - hit_at[2];
            let d2 = dx * dx + dy * dy + dz * dz;
            if d2 <= 900.0 && best.is_none_or(|(_, best_d2, _)| d2 < best_d2) {
                best = Some((i, d2, *b));
            }
        }
        best
    };

    let Some((idx, _, parent)) = candidate else {
        return false;
    };

    let vel = parent.entity.velocity(host).unwrap_or([
        hit_at[0] - parent.prev_pos[0],
        hit_at[1] - parent.prev_pos[1],
        hit_at[2] - parent.prev_pos[2],
    ]);
    let speed = (vel[0] * vel[0] + vel[1] * vel[1] + vel[2] * vel[2]).sqrt();
    let next_speed = speed * LEAF_VELOCITY_RETENTION;
    if !next_speed.is_finite() || next_speed < MIN_LEAF_PERFORATION_SPEED || speed < 1e-4 {
        return false;
    }

    let Some(exit_pos) = compute_voxel_ray_exit(block, hit_at, vel) else {
        return false;
    };
    let Some(owner_ent) = host.entity(owner_eid) else {
        return false;
    };
    let Ok(bullet_id) = Identifier::parse(BULLET) else {
        return false;
    };
    let Some(item) = host.item_key(&bullet_id) else {
        return false;
    };
    let Some(carried) = ItemStack::new(item, 1) else {
        return false;
    };

    let next_vel = [
        vel[0] * LEAF_VELOCITY_RETENTION,
        vel[1] * LEAF_VELOCITY_RETENTION,
        vel[2] * LEAF_VELOCITY_RETENTION,
    ];
    let owner_living = LivingRef::new(owner_ent);
    let Some(child) =
        host.spawn_projectile_from_rotation(owner_living, &bullet_id, &carried, 0.0, 0.0, 0.0)
    else {
        return false;
    };
    let child_ent = child.entity();
    if !child_ent.teleport_to(host, exit_pos) || !child_ent.set_velocity(host, next_vel) {
        let _ = child_ent.discard(host);
        return false;
    }

    if let Some(slot) = ACTIVE_BULLETS.lock().unwrap().get_mut(idx) {
        slot.terminated = true;
    }
    let next_pierces = parent.leaf_pierces.saturating_add(1);
    register_spawned_bullet(
        host,
        child_ent,
        owner_eid,
        parent.shooter_uuid,
        parent.origin,
        false,
        next_pierces,
        parent.whizzed_eids,
        now,
    );
    if parent.shooter_uuid != 0 {
        record_bullet_shot(child_ent.id(), owner_eid, parent.shooter_uuid, parent.origin, now);
    }
    true
}

/// Steps post-shot barrel heat cooling, rising muzzle smoke wisps after sustained rapid fire,
/// and steam quenching (`SND_LAVA_EXTINGUISH`) when a hot barrel dips into water.
pub(crate) fn tick_barrel_heat(host: &mut dyn ServerHost, now: u64) {
    let uuids: Vec<u128> = {
        let guard = BARREL_HEAT.lock().unwrap();
        if guard.is_empty() {
            return;
        }
        guard.keys().copied().collect()
    };

    for uuid in uuids {
        let Some(player) = host.player(uuid) else {
            BARREL_HEAT.lock().unwrap().remove(&uuid);
            continue;
        };
        let Some(level) = player.entity().level(host) else {
            continue;
        };
        if !level.in_place(host) {
            continue;
        }

        let Some(mut state) = BARREL_HEAT.lock().unwrap().get(&uuid).copied() else {
            continue;
        };
        if state.last_step_tick >= now {
            continue;
        }
        let dt = now.saturating_sub(state.last_step_tick);
        state.heat = step_barrel_heat(state.heat, dt);
        state.last_step_tick = now;

        if state.heat > 0.0
            && held_rifle(host, player).is_some()
            && let Some(frame) = compute_shooter_frame(host, player, state.muzzle_mm)
        {
            if is_point_in_water(host, frame.level, frame.muzzle) {
                if state.heat >= 0.45 {
                    frame.level.play_sound(
                        host,
                        SND_LAVA_EXTINGUISH,
                        frame.muzzle,
                        SoundSource::Players,
                        0.38,
                        1.65,
                    );
                    if let Some(white_smoke) = WHITE_SMOKE_KEY.get().copied() {
                        let steam_opts = ParticleOptions::simple(white_smoke);
                        frame.level.send_particles(
                            host,
                            &steam_opts,
                            None,
                            frame.muzzle,
                            [0.03, 0.04, 0.03],
                            0.015,
                            4,
                        );
                    }
                    if let Some(bubble) = BUBBLE_KEY.get().copied() {
                        let bubble_opts = ParticleOptions::simple(bubble);
                        frame.level.send_particles(
                            host,
                            &bubble_opts,
                            None,
                            frame.muzzle,
                            [0.03, 0.03, 0.03],
                            0.04,
                            5,
                        );
                    }
                }
                state.heat = 0.0;
            } else if state.heat >= BARREL_HEAT_WISP_THRESHOLD
                && now.saturating_sub(state.last_shot_tick) >= 5
                && (now.wrapping_add(uuid as u64 & 3)) % 3 == 0
                && let Some(smoke) = SMOKE_KEY.get().copied()
            {
                let smoke_opts = ParticleOptions::simple(smoke);
                frame.level.send_particles(
                    host,
                    &smoke_opts,
                    None,
                    frame.muzzle,
                    [0.008, 0.014, 0.008],
                    0.006,
                    1,
                );
            }
        }

        if state.heat <= 0.0 {
            BARREL_HEAT.lock().unwrap().remove(&uuid);
        } else {
            BARREL_HEAT.lock().unwrap().insert(uuid, state);
        }
    }
}

/// Flushes any pending water-surface entry splash & cavitation streak if a bullet enters shallow
/// water and impacts the riverbed during the exact same server tick (`projectile_impact` runs
/// before `level_tick_post`).
pub(crate) fn consume_bullet_water_splash_on_impact(
    host: &mut dyn ServerHost,
    level: LevelRef,
    hit_at: [f64; 3],
) {
    let mut splashed_entry: Option<[f64; 3]> = None;
    {
        let mut active = ACTIVE_BULLETS.lock().unwrap();
        for b in active.iter_mut() {
            if let Some(entry) = b.pending_splash {
                let dx = entry[0] - hit_at[0];
                let dy = entry[1] - hit_at[1];
                let dz = entry[2] - hit_at[2];
                if dx * dx + dy * dy + dz * dz <= 36.0 {
                    splashed_entry = b.pending_splash.take();
                    break;
                }
            }
        }
    }

    if let Some(entry_pos) = splashed_entry {
        emit_water_entry_splash(host, level, entry_pos);
        emit_cavitation_trail(host, level, entry_pos, hit_at);
    }
}

pub(crate) fn tick_active_bullets(host: &mut dyn ServerHost, now: u64) {
    let bullets = {
        let mut guard = ACTIVE_BULLETS.lock().unwrap();
        if guard.is_empty() {
            return;
        }
        core::mem::take(&mut *guard)
    };

    let mut kept = Vec::with_capacity(bullets.len());
    for mut b in bullets {
        if b.terminated || now.saturating_sub(b.spawn_tick) > 120 || !host.entity_is_valid(b.entity) {
            continue;
        }
        let Some(level) = b.entity.level(host) else {
            continue;
        };
        if !level.in_place(host) || b.last_step_tick >= now {
            kept.push(b);
            continue;
        }
        b.last_step_tick = now;

        let (Some(pos), Some(vel)) = (b.entity.position(host), b.entity.velocity(host)) else {
            continue;
        };

        if let Some(entry_pos) = b.pending_splash.take() {
            emit_water_entry_splash(host, level, entry_pos);
            emit_cavitation_trail(host, level, entry_pos, pos);
            b.in_water = true;
        } else if b.in_water {
            emit_cavitation_trail(host, level, b.prev_pos, pos);
        }

        if is_point_in_water(host, level, pos) {
            b.in_water = true;
            if let Some(next_vel) = step_underwater_bullet_velocity(vel) {
                let _ = b.entity.set_velocity(host, next_vel);
            } else {
                if let Some(bubble) = BUBBLE_KEY.get().copied() {
                    let bubble_opts = ParticleOptions::simple(bubble);
                    level.send_particles(
                        host,
                        &bubble_opts,
                        None,
                        pos,
                        [0.03, 0.03, 0.03],
                        0.02,
                        3,
                    );
                }
                let _ = b.entity.discard(host);
                continue;
            }
        } else {
            b.in_water = false;
            let seg_a = b.prev_pos;
            emit_segment_flybys(host, level, &mut b, seg_a, pos, None, false);
            let speed = (vel[0] * vel[0] + vel[1] * vel[1] + vel[2] * vel[2]).sqrt();
            if speed.is_finite() && speed > 1e-4 {
                let dir = [vel[0] / speed, vel[1] / speed, vel[2] / speed];
                if let Some((s_surf, entry_pos)) =
                    find_water_surface_along_ray(host, level, pos, dir, speed)
                {
                    let step_dist = compute_water_entry_step(speed, s_surf);
                    let _ = b.entity.set_velocity(
                        host,
                        [dir[0] * step_dist, dir[1] * step_dist, dir[2] * step_dist],
                    );
                    b.pending_splash = Some(entry_pos);
                }
            }
        }

        b.prev_pos = pos;
        kept.push(b);
    }

    let mut guard = ACTIVE_BULLETS.lock().unwrap();
    kept.append(&mut *guard);
    *guard = kept;
}

/// Reads the Winchester strictly from the player's currently selected hotbar slot (`0..9`).
fn held_rifle(host: &mut dyn ServerHost, player: mod_api::PlayerRef) -> Option<(u32, ItemStack)> {
    let sel = player.selected_slot(host)? as u32;
    let slots = player.inventory_slots(host)?;
    for (slot, stack) in slots {
        if slot == sel && stack.item().id(host).to_string() == WINCHESTER {
            return Some((slot, stack));
        }
    }
    None
}

fn has_cartridge(host: &mut dyn ServerHost, player: mod_api::PlayerRef) -> bool {
    let Some(slots) = player.inventory_slots(host) else {
        return false;
    };
    slots
        .into_iter()
        .any(|(_, stack)| stack.item().id(host).to_string() == CARTRIDGE && stack.count() > 0)
}

fn take_cartridge(host: &mut dyn ServerHost, player: mod_api::PlayerRef) -> bool {
    let Some(slots) = player.inventory_slots(host) else {
        return false;
    };
    for (slot, stack) in slots {
        if stack.item().id(host).to_string() != CARTRIDGE {
            continue;
        }
        let count = stack.count();
        let write = if count > 1 {
            let mut rest = stack;
            if rest.set_count(count - 1).is_err() {
                return false;
            }
            Some(rest)
        } else {
            None
        };
        return player.set_inventory_slot(host, slot, write).is_ok();
    }
    false
}

fn compute_spread(host: &dyn ServerHost, player: mod_api::PlayerRef, now: u64) -> f32 {
    let entity = player.entity();
    let crouching = entity.is_crouching(host).unwrap_or(false);
    let sprinting = entity.is_sprinting(host).unwrap_or(false);
    let on_ground = entity.on_ground(host).unwrap_or(true);
    let horiz_speed_sq = entity
        .velocity(host)
        .map_or(0.0, |[vx, _, vz]| vx * vx + vz * vz);
    let moving = sprinting || !on_ground || horiz_speed_sq > 0.012;

    let hip = if moving {
        SPREAD_HIP_MOVING
    } else if crouching {
        SPREAD_HIP_CROUCH
    } else {
        SPREAD_HIP_STAND
    };

    let (held, last_seen) = AIM_TICKS
        .lock()
        .unwrap()
        .get(&player.uuid())
        .copied()
        .unwrap_or((0, 0));
    if held <= 0 || now.saturating_sub(last_seen) > 2 {
        return hip;
    }

    let ads = if moving {
        SPREAD_ADS_MOVING
    } else if crouching {
        SPREAD_ADS_CROUCH
    } else {
        SPREAD_ADS_STAND
    };

    if held >= ADS_SETTLE_TICKS {
        ads
    } else {
        let frac = (held as f32) / (ADS_SETTLE_TICKS as f32);
        hip + (ads - hip) * frac
    }
}

fn fire_rifle(
    host: &mut dyn ServerHost,
    player: mod_api::PlayerRef,
    smoke: ParticleKey,
    muzzle_mm: [i32; 3],
    aim_pct: i32,
) {
    let Some((slot, rifle)) = held_rifle(host, player) else {
        return;
    };
    let uuid = player.uuid();
    let now = SERVER_TICKS.load(Ordering::Relaxed);
    let loaded = rounds(&rifle);

    // Firing mid-reload interrupts the reload immediately if >= 1 round is in the tube.
    if is_player_reloading(uuid) {
        let _ = cancel_reload_session(host, player);
        if loaded == 0 {
            send_sync(host, player, SYNC_DRY_CLICK, 0);
            broadcast_tp_state(host, player, TP_CUE_DRY_CLICK);
            return;
        }
    }

    let lever_ready = LEVER_READY
        .lock()
        .unwrap()
        .get(&uuid)
        .copied()
        .unwrap_or(0);
    if lever_ready > now {
        return;
    }

    if loaded == 0 {
        LEVER_READY.lock().unwrap().insert(uuid, now + 5);
        send_sync(host, player, SYNC_DRY_CLICK, 0);
        broadcast_tp_state(host, player, TP_CUE_DRY_CLICK);
        return;
    }

    let frame = compute_shooter_frame(host, player, muzzle_mm);
    if let Some(ref f) = frame
        && is_eye_or_breech_submerged(host, player, f)
    {
        // Flooded receiver/breech: blackpowder `.44-40` cartridge fails to ignite underwater
        // without wasting the round; vents small air bubbles with a damp mechanical click.
        LEVER_READY.lock().unwrap().insert(uuid, now + 7);
        emit_wet_misfire(host, player, f, loaded);
        grant_mod_criterion(host, player, TRIG_WET_POWDER);
        return;
    }

    let muzzle_in_water = frame
        .as_ref()
        .is_some_and(|f| is_point_in_water(host, f.level, f.muzzle));

    let Ok(bullet) = Identifier::parse(BULLET) else {
        return;
    };
    let Some(item) = host.item_key(&bullet) else {
        return;
    };
    let Some(carried) = ItemStack::new(item, 1) else {
        return;
    };
    let owner = LivingRef::new(player.entity());
    let spread = compute_spread(host, player, now);
    let Some(shot) =
        host.spawn_projectile_from_rotation(owner, &bullet, &carried, 0.0, BULLET_SPEED, spread)
    else {
        return;
    };

    let shot_origin = frame
        .as_ref()
        .map(|f| f.muzzle)
        .or_else(|| player.entity().eye_position(host))
        .or_else(|| player.entity().position(host));
    register_spawned_bullet(
        host,
        shot.entity(),
        player.entity().id(),
        uuid,
        shot_origin.unwrap_or([0.0; 3]),
        muzzle_in_water,
        0,
        [0; 4],
        now,
    );
    if let Some(origin) = frame
        .as_ref()
        .map(|f| f.eye)
        .or_else(|| player.entity().eye_position(host))
        .or_else(|| player.entity().position(host))
    {
        record_bullet_shot(shot.entity().id(), player.entity().id(), uuid, origin, now);
    }

    // Emit Sculk/Warden `minecraft:projectile_shoot` vibration attributed directly to the shooter.
    if let Some(level) = frame
        .as_ref()
        .map(|f| f.level)
        .or_else(|| player.entity().level(host))
        && let Some(at) = shot_origin
    {
        let _ = level.game_event(
            host,
            GAME_EVENT_PROJECTILE_SHOOT,
            at,
            GameEventContext {
                source_entity: Some(player.entity()),
                affected_state: None,
            },
        );
    }

    // Accumulate barrel heat across rapid shots, or quench immediately if the crown is underwater.
    {
        let mut heat_map = BARREL_HEAT.lock().unwrap();
        let prev_heat = heat_map
            .get(&uuid)
            .map_or(0.0, |st| step_barrel_heat(st.heat, now.saturating_sub(st.last_step_tick)));
        if muzzle_in_water {
            heat_map.remove(&uuid);
            drop(heat_map);
            if prev_heat >= 0.45
                && let Some(ref f) = frame
            {
                f.level.play_sound(
                    host,
                    SND_LAVA_EXTINGUISH,
                    f.muzzle,
                    SoundSource::Players,
                    0.42,
                    1.60,
                );
            }
        } else {
            heat_map.insert(
                uuid,
                BarrelHeatState {
                    heat: add_shot_barrel_heat(prev_heat),
                    last_shot_tick: now,
                    last_step_tick: now,
                    muzzle_mm,
                },
            );
        }
    }

    let remaining = loaded - 1;
    let mut rifle = rifle;
    rifle.set_damage(remaining);
    let _ = player.set_inventory_slot(host, slot, Some(rifle));

    let next_ready = if remaining > 0 {
        now + LEVER_TICKS
    } else {
        now + 6
    };
    LEVER_READY.lock().unwrap().insert(uuid, next_ready);

    // Mono 48kHz gunshot blast + breech thump broadcast in 3D world space for all nearby players.
    // Subtle shot-to-shot pitch jitter prevents identical waveform repetition at 120 RPM.
    let j = shot_pitch_jitter(now, uuid);
    if muzzle_in_water {
        play_world_sound(host, player, SND_FIRE, 1.65, 0.78 + 0.022 * j);
        play_world_sound(host, player, SND_THUMP, 0.95, 0.86 + 0.020 * j);
        if let Some(ref f) = frame {
            f.level
                .play_sound(host, SND_WATER_SPLASH, f.muzzle, SoundSource::Players, 0.85, 0.92 + 0.03 * j);
        }
    } else {
        play_world_sound(host, player, SND_FIRE, 2.5, 1.0 + 0.028 * j);
        play_world_sound(host, player, SND_THUMP, 1.15, 1.0 + 0.022 * j);
        if let Some(ref f) = frame {
            probe_and_broadcast_shot_acoustics(host, player, f.muzzle, f.level);
        }
    }
    if let Some(ref f) = frame {
        muzzle_effects(host, f, smoke, aim_pct, muzzle_in_water);
    }
    send_sync(host, player, SYNC_SHOT, remaining);
    if remaining > 0 {
        broadcast_tp_state(host, player, TP_STATE_LEVER_CYCLE);
    } else {
        broadcast_tp_state(host, player, TP_CUE_LAST_SHOT_EJECT);
    }
}

fn start_reload(host: &mut dyn ServerHost, player: mod_api::PlayerRef) {
    let Some((slot, rifle)) = held_rifle(host, player) else {
        return;
    };
    let uuid = player.uuid();
    if is_player_reloading(uuid) {
        return;
    }
    if player.entity().is_sprinting(host).unwrap_or(false) {
        return;
    }

    let loaded = rounds(&rifle);
    if loaded >= TUBE_CAPACITY {
        return;
    }

    // Cannot open the side loading gate to feed blackpowder `.44-40` rounds while submerged.
    if let Some(f) = compute_shooter_frame(host, player, [0; 3])
        && is_eye_or_breech_submerged(host, player, &f)
    {
        emit_wet_misfire(host, player, &f, loaded);
        return;
    }

    let infinite = player.has_infinite_materials(host).unwrap_or(false);
    if !infinite && !has_cartridge(host, player) {
        send_sync(host, player, SYNC_DRY_CLICK, loaded);
        return;
    }

    AIM_TICKS.lock().unwrap().remove(&uuid);
    let now = SERVER_TICKS.load(Ordering::Relaxed);
    RELOAD_SESSIONS.lock().unwrap().insert(
        uuid,
        ReloadSession {
            slot,
            from_empty: loaded == 0,
            inserted_any: false,
            phase: ReloadPhase::Opening {
                until: now + RELOAD_OPEN_TICKS,
            },
        },
    );
    send_sync(host, player, SYNC_RELOAD_START, loaded);
    broadcast_tp_state(host, player, TP_STATE_RELOADING);
}

pub(crate) fn tick_reload_sessions(host: &mut dyn ServerHost, now: u64) {
    let uuids: Vec<u128> = RELOAD_SESSIONS.lock().unwrap().keys().copied().collect();
    for uuid in uuids {
        let Some(entity) = host.entity_by_uuid(EntityUuid(uuid)) else {
            continue;
        };
        let Some(player) = host.player(uuid) else {
            RELOAD_SESSIONS.lock().unwrap().remove(&uuid);
            continue;
        };
        let Some(mut session) = RELOAD_SESSIONS.lock().unwrap().get(&uuid).copied() else {
            continue;
        };

        let sprinting = entity.is_sprinting(host).unwrap_or(false);
        let submerged = is_player_submerged_simple(host, player);
        let valid_rifle = held_rifle(host, player).filter(|(sel, _)| *sel == session.slot);

        let Some((slot, rifle)) = valid_rifle else {
            RELOAD_SESSIONS.lock().unwrap().remove(&uuid);
            send_sync(host, player, SYNC_RELOAD_ABORTED, 0);
            broadcast_tp_state(host, player, TP_STATE_IDLE);
            continue;
        };
        let loaded = rounds(&rifle);
        if sprinting || submerged {
            RELOAD_SESSIONS.lock().unwrap().remove(&uuid);
            send_sync(host, player, SYNC_RELOAD_ABORTED, loaded);
            broadcast_tp_state(host, player, TP_STATE_IDLE);
            continue;
        }

        let mut remove_session = false;
        match session.phase {
            ReloadPhase::Opening { until } => {
                if now >= until {
                    session.phase = ReloadPhase::Inserting { next_tick: now };
                }
            }
            ReloadPhase::Inserting { next_tick } => {
                if now >= next_tick {
                    let infinite = player.has_infinite_materials(host).unwrap_or(false);
                    if loaded >= TUBE_CAPACITY || (!infinite && !take_cartridge(host, player)) {
                        let empty_chamber =
                            session.from_empty && session.inserted_any && loaded > 0;
                        let close_ticks = if empty_chamber {
                            RELOAD_CLOSE_EMPTY_TICKS
                        } else {
                            RELOAD_CLOSE_NORM_TICKS
                        };
                        session.phase = ReloadPhase::Closing {
                            until: now + close_ticks,
                            empty_chamber,
                        };
                        send_sync(
                            host,
                            player,
                            if empty_chamber {
                                SYNC_RELOAD_END_EMPTY
                            } else {
                                SYNC_RELOAD_END_NORM
                            },
                            loaded,
                        );
                        broadcast_tp_state(
                            host,
                            player,
                            if empty_chamber {
                                TP_CUE_EMPTY_CHAMBER_LEVER
                            } else {
                                TP_CUE_RELOAD_END_NORM
                            },
                        );
                    } else {
                        let next_loaded = loaded + 1;
                        let mut rifle = rifle;
                        rifle.set_damage(next_loaded);
                        if player.set_inventory_slot(host, slot, Some(rifle)).is_err() {
                            remove_session = true;
                            send_sync(host, player, SYNC_RELOAD_ABORTED, loaded);
                            broadcast_tp_state(host, player, TP_STATE_IDLE);
                        } else {
                            session.inserted_any = true;
                            send_sync(host, player, SYNC_SHELL_INSERTED, next_loaded);
                            broadcast_tp_state(host, player, TP_CUE_RELOAD_INSERT);
                            if next_loaded == TUBE_CAPACITY {
                                grant_mod_criterion(host, player, TRIG_FULL_TUBE);
                            }
                            // Wait `RELOAD_INSERT_TICKS` so the shell-insertion animation completes
                            // before transitioning to either the next shell or `ReloadPhase::Closing`.
                            session.phase = ReloadPhase::Inserting {
                                next_tick: now + RELOAD_INSERT_TICKS,
                            };
                        }
                    }
                }
            }
            ReloadPhase::Closing {
                until,
                empty_chamber,
            } => {
                if now >= until {
                    remove_session = true;
                    LEVER_READY
                        .lock()
                        .unwrap()
                        .insert(uuid, now + if empty_chamber { 2 } else { 1 });
                }
            }
        }

        if remove_session {
            RELOAD_SESSIONS.lock().unwrap().remove(&uuid);
        } else {
            RELOAD_SESSIONS.lock().unwrap().insert(uuid, session);
        }
    }
}

#[mod_api::horizon_part]
impl ItemPart for Winchester {
    fn use_item(
        &self,
        _: HostSide<'_>,
        event: &mut UseItemEvent<'_>,
    ) -> Option<InteractionResult> {
        if event.hand == Hand::Off {
            // Slung diagonally across the back when in the off-hand: never enter ADS from off-hand.
            return Some(InteractionResult::Fail);
        }
        None
    }

    fn use_duration(&self, _: &dyn Host, event: &mut UseDurationEvent<'_>) {
        event.duration = USE_DURATION;
    }

    fn item_bar(&self, _: &dyn ClientHost, event: &mut ItemBarEvent<'_>) {
        let loaded = rounds(&event.stack);
        let width = (loaded * 13 / TUBE_CAPACITY) as i32;
        *event.bar = Some((width, crate::settings::client_settings().hud_accent_color));
    }

    /// Suppress vanilla's equip dip whenever ammo count (`damage`) updates in the same slot.
    fn should_cause_reequip_animation(
        &self,
        _: &dyn ClientHost,
        event: &mut ShouldCauseReequipAnimationEvent<'_>,
    ) {
        event.reequip = event.slot_changed;
    }

    fn on_use_tick(&self, host: HostSide<'_>, event: &mut OnUseTickEvent<'_>) {
        match host {
            HostSide::Client(_) => {
                if !CLIENT_RELOADING.load(Ordering::Relaxed)
                    && !CLIENT_SPRINTING.load(Ordering::Relaxed)
                {
                    AIMING.store(true, Ordering::Relaxed);
                    AIM_LAST_TICK.store(CLIENT_TICKS.load(Ordering::Relaxed), Ordering::Relaxed);
                }
            }
            HostSide::Server(host) => {
                let Some(player) = shooter(host) else {
                    return;
                };
                let uuid = player.uuid();
                if is_player_reloading(uuid) {
                    AIM_TICKS.lock().unwrap().remove(&uuid);
                    return;
                }
                let held = USE_DURATION - event.remaining;
                let now = SERVER_TICKS.load(Ordering::Relaxed);
                AIM_TICKS.lock().unwrap().insert(uuid, (held, now));
            }
        }
    }

    /// Stack `damage` stores loaded `.44-40` rounds (`0..=8`), not wear & tear:
    /// disable Mending XP consumption (`ratio = 0.0`) so XP orbs never delete loaded rounds.
    fn xp_repair_ratio(&self, _: &dyn Host, event: &mut XpRepairRatioEvent<'_>) {
        event.ratio = 0.0;
    }

    /// Prevent Grindstone two-item combination from destroying a second Winchester or wiping ammo,
    /// while preserving single-item Grindstone disenchanting.
    fn can_grindstone_repair(&self, _: &dyn LoopContext, event: &mut CanGrindstoneRepairEvent<'_>) {
        event.can_repair = false;
    }

    /// Prevent Crafting Table, 2×2 inventory grid, and Redstone Crafter item-combination repair
    /// from merging two Winchesters and wiping their loaded `.44-40` rounds.
    fn is_combine_repairable(&self, _: &dyn LoopContext, event: &mut IsCombineRepairableEvent<'_>) {
        event.repairable = false;
    }

    fn release_using(&self, host: HostSide<'_>, event: &mut ReleaseUsingEvent<'_>) {
        event.used = true;
        match host {
            HostSide::Client(_) => AIMING.store(false, Ordering::Relaxed),
            HostSide::Server(host) => {
                if let Some(player) = shooter(host) {
                    AIM_TICKS.lock().unwrap().remove(&player.uuid());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn barrel_heat_accumulates_on_rapid_fire_and_cools_smoothly() {
        let h1 = add_shot_barrel_heat(0.0);
        assert!(h1 < BARREL_HEAT_WISP_THRESHOLD, "single shot should not smoke: {h1}");
        let h2 = add_shot_barrel_heat(step_barrel_heat(h1, 13));
        let h3 = add_shot_barrel_heat(step_barrel_heat(h2, 13));
        assert!(
            h3 >= BARREL_HEAT_WISP_THRESHOLD,
            "rapid 3-shot burst should cross smoke wisp threshold: {h3}"
        );
        assert_eq!(step_barrel_heat(h3, 200), 0.0);
    }

    #[test]
    fn water_entry_step_caps_first_tick_underwater_penetration() {
        let speed = f64::from(BULLET_SPEED); // 15.0 b/t
        // Entering water 4.0 blocks away: 4.0m air + min((11.0 * 0.16), 2.10) = 5.76m total (< 6.10m).
        let step = compute_water_entry_step(speed, 4.0);
        assert!(step > 5.70 && step < 5.80, "step={step}");
        assert!(step - 4.0 <= WATER_FIRST_TICK_PENETRATION);

        // Point-blank surface entry (`surface_dist = 0.2m`): capped at `0.2 + 2.10 = 2.30m`.
        let close = compute_water_entry_step(speed, 0.2);
        assert!((close - 2.30).abs() < 1e-6, "close={close}");
    }

    #[test]
    fn underwater_bullet_velocity_stops_within_realistic_depth() {
        // Simulate post-entry velocity (`~1.8` blocks/tick underwater) across ticks with vanilla
        // 0.8x water drag + our `step_underwater_bullet_velocity` until spent (`None`).
        let mut vel = [0.0_f64, 0.0, 1.80];
        let mut extra_depth = 0.0_f64;
        let mut ticks = 0_u32;
        while let Some(next) = step_underwater_bullet_velocity(vel) {
            ticks += 1;
            // Vanilla ThrowableProjectile applies ~0.8x in water on the next tick.
            vel = [next[0] * 0.8, next[1] * 0.8, next[2] * 0.8];
            extra_depth += (vel[0] * vel[0] + vel[1] * vel[1] + vel[2] * vel[2]).sqrt();
            assert!(ticks <= 8, "bullet failed to stop underwater");
        }
        assert!((2..=5).contains(&ticks), "ticks={ticks}");
        // Total penetration (<= 2.10m entry + extra_depth) stays around ~2.8m..3.6m (~9..12 ft).
        let total_depth = WATER_FIRST_TICK_PENETRATION + extra_depth;
        assert!(
            (2.5..=4.0).contains(&total_depth),
            "unexpected total underwater penetration: {total_depth}"
        );
    }

    #[test]
    fn bullet_flyby_waits_for_closest_approach_and_splits_supersonic_vs_subsonic() {
        let muzzle = [0.0, 64.0, 0.0];
        let listener = [0.6, 64.0, 20.0];

        // Step 1 (`0 -> 15m`): bullet is still 5m in front of listener (`ahead_dot > 0`) -> defer!
        assert_eq!(
            evaluate_bullet_flyby(
                muzzle,
                [0.0, 64.0, 0.0],
                [0.0, 64.0, 15.0],
                listener,
                false
            ),
            None
        );

        // Step 2 (`15 -> 30m`): crosses listener plane at `z = 20.0`, `miss = 0.6m`, `travel = 20m` -> crack!
        let hit = evaluate_bullet_flyby(
            muzzle,
            [0.0, 64.0, 15.0],
            [0.0, 64.0, 30.0],
            listener,
            false,
        )
        .expect("expected supersonic flyby");
        assert!(hit.is_supersonic_crack);
        assert!((distance_3d(listener, hit.closest_point) - 0.6).abs() < 1e-6);
        assert!((hit.closest_point[2] - 20.0).abs() < 1e-6);
        assert!(hit.proximity > 0.85);

        // Downrange (`55m > 38m`) close pass (`1.1m`): transonic/subsonic Doppler lead whiz!
        let far_whiz = evaluate_bullet_flyby(
            muzzle,
            [0.0, 64.0, 45.0],
            [0.0, 64.0, 60.0],
            [1.1, 64.0, 55.0],
            false,
        )
        .expect("expected downrange whiz");
        assert!(!far_whiz.is_supersonic_crack);

        // Point-blank (`1.2m < 2.6m` from muzzle) never triggers a flyby on bystanders hugging barrel.
        assert_eq!(
            evaluate_bullet_flyby(
                muzzle,
                [0.0, 64.0, 0.0],
                [0.0, 64.0, 15.0],
                [0.5, 64.0, 1.2],
                false
            ),
            None
        );

        // Terminal wall hit 4.5m in front of listener (`ahead_dot > 0` and `miss > 2.75m`) is ignored,
        // whereas a wall hit 0.9m right behind the listener's head triggers a crisp snap.
        assert_eq!(
            evaluate_bullet_flyby(
                muzzle,
                [0.0, 64.0, 5.0],
                [0.0, 64.0, 15.5],
                listener,
                true
            ),
            None
        );
        let wall_behind = evaluate_bullet_flyby(
            muzzle,
            [0.0, 64.0, 15.0],
            [0.0, 64.0, 20.9],
            listener,
            true,
        )
        .expect("expected terminal flyby on wall right behind listener");
        assert!(wall_behind.is_supersonic_crack);
    }

    #[test]
    fn shot_acoustics_and_entity_impact_classifiers_cover_all_regimes() {
        let (cave_cue, cave_int) = classify_shot_acoustics(false, 0, true, true, 3, 4);
        assert_eq!(cave_cue, ACOUSTIC_CUE_SHOT_CONFINED);
        assert!(cave_int >= 850);

        let (porch_cue, _) = classify_shot_acoustics(false, 12, false, true, 0, 2);
        assert_eq!(porch_cue, ACOUSTIC_CUE_SHOT_SEMI);

        let (plains_cue, plains_int) = classify_shot_acoustics(true, 15, false, false, 0, 0);
        let (canyon_cue, canyon_int) = classify_shot_acoustics(true, 15, false, false, 1, 4);
        assert_eq!(plains_cue, ACOUSTIC_CUE_SHOT_OPEN);
        assert_eq!(canyon_cue, ACOUSTIC_CUE_SHOT_OPEN);
        assert!(canyon_int > plains_int + 300);

        assert_eq!(
            classify_entity_impact_kind("minecraft:player", Some("minecraft:iron_chestplate"), false),
            EntityImpactKind::ArmorMetal
        );
        assert_eq!(
            classify_entity_impact_kind("minecraft:player", Some("minecraft:leather_chestplate"), false),
            EntityImpactKind::Flesh
        );
        assert_eq!(
            classify_entity_impact_kind("minecraft:player", None, true),
            EntityImpactKind::ShieldBlock
        );
        assert_eq!(
            classify_entity_impact_kind("minecraft:iron_golem", None, false),
            EntityImpactKind::ArmorMetal
        );
        assert_eq!(
            classify_entity_impact_kind("minecraft:skeleton", None, false),
            EntityImpactKind::RigidConstruct
        );

        for coord in [-29_999_984.25_f64, -123.4375, 0.0, 42.6875, 29_999_984.5] {
            let roundtrip = unpack_coord_sub16(pack_coord_sub16(coord));
            assert!((roundtrip - coord).abs() <= 0.0625 + 1e-6);
        }
    }
}
