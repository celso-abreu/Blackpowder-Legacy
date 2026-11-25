//! Server-authoritative `.44-40` firing, stance spread, muzzle VFX, submerged blackpowder
//! misfire & hydrodynamic bullet drag, per-shell side-gate reload loop, and `ItemPart` hooks.

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use mod_api::{
    ClientHost, EntityRef, EntityUuid, Hand, Host, HostSide, IPayloadContext, Identifier,
    InteractionResult, ItemBarEvent, ItemPart, ItemStack, LevelRef, LivingRef, OnUseTickEvent,
    ParticleKey, ParticleOptions, ReleaseUsingEvent, ServerHost, ShouldCauseReequipAnimationEvent,
    SoundSource, UseDurationEvent, UseItemEvent, VarInt,
};

use crate::constants::*;
use crate::exploration::{grant_mod_criterion, record_bullet_shot};
use crate::skin::{default_skin_for_uuid, parse_mojang_skin_property};
use crate::{
    is_point_in_water, RifleInput, RifleSkinSync, RifleSync, RifleThirdPersonSync, AIMING,
    AIM_LAST_TICK, BUBBLE_KEY, BUBBLE_POP_KEY, CLIENT_RELOADING, CLIENT_SPRINTING, CLIENT_TICKS,
    FLAME_KEY, RIFLE_SKIN_SYNC, RIFLE_SYNC, RIFLE_TP_SYNC, SERVER_TICKS, SMOKE_KEY, SPLASH_KEY,
    WHITE_SMOKE_KEY,
};

static AIM_TICKS: Mutex<BTreeMap<u128, (i32, u64)>> = Mutex::new(BTreeMap::new());
static LEVER_READY: Mutex<BTreeMap<u128, u64>> = Mutex::new(BTreeMap::new());
static RELOAD_SESSIONS: Mutex<BTreeMap<u128, ReloadSession>> = Mutex::new(BTreeMap::new());
static ACTIVE_BULLETS: Mutex<Vec<ActiveBullet>> = Mutex::new(Vec::new());

#[derive(Clone, Copy)]
struct ActiveBullet {
    entity: EntityRef,
    prev_pos: [f64; 3],
    in_water: bool,
    pending_splash: Option<[f64; 3]>,
    spawn_tick: u64,
    last_step_tick: u64,
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

fn register_spawned_bullet(
    host: &mut dyn ServerHost,
    entity: EntityRef,
    muzzle_in_water: bool,
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
        prev_pos: pos0,
        in_water: starts_in_water,
        pending_splash,
        spawn_tick: now,
        last_step_tick: now,
    });
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
        if now.saturating_sub(b.spawn_tick) > 120 || !host.entity_is_valid(b.entity) {
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

    register_spawned_bullet(host, shot.entity(), muzzle_in_water, now);
    if let Some(origin) = frame
        .as_ref()
        .map(|f| f.eye)
        .or_else(|| player.entity().eye_position(host))
        .or_else(|| player.entity().position(host))
    {
        record_bullet_shot(shot.entity().id(), player.entity().id(), uuid, origin, now);
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
    }
    if let Some(ref f) = frame {
        muzzle_effects(host, f, smoke, aim_pct, muzzle_in_water);
    }
    send_sync(host, player, SYNC_SHOT, remaining);
    if remaining > 0 {
        broadcast_tp_state(host, player, TP_STATE_LEVER_CYCLE);
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
                                TP_STATE_LEVER_CYCLE
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
}
