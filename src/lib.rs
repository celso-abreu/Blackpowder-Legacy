//! Just a Rifle — Winchester 1873 lever-action rifle for Horizon (wasm mod, v0.11.0).
//!
//! Clean-room Rust `v0.11.0` modular architecture:
//! - `constants`   — Content IDs, ballistics, and exact Blockbench `[0..16]` texel pivots.
//! - `ballistics`  — Sub-tick 3D ray-AABB headshot geometry, shield wear & leaf-pierce math.
//! - `settings`    — `Options > Mods > Config` 33-setting registry & per-channel audio mixer.
//! - `skin`        — Mojang Base64 + UTF-16LE SHA-1 skin decoder & 18-skin UUID fallback.
//! - `arms`        — 1st-person `slim` (`3×12×4`) / `wide` (`4×12×4`) arm & 3D sleeve meshes.
//! - `pose`        — 2nd-order springs, TAC state machine, dual-arm poses & sight kinematics.
//! - `world_fx`    — Reactive world targets, TNT/foliage ballistics, bullet decals & casings.
//! - `exploration` — Structure chest loot, Illager `.44-40` drops & 14 mod advancements.
//! - `server`      — Authoritative firing, barrel heat, foliage perforation & reload loop.

mod arms;
mod ballistics;
mod constants;
mod exploration;
mod pose;
mod server;
mod settings;
mod skin;
mod world_fx;

use std::collections::BTreeMap;
use std::f32::consts::{PI, TAU};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use mod_api::{
    AnvilUpdateEvent, ArmPose, ArmPoseTransformEvent, BlockKey, CalculatePlayerTurnEvent,
    CameraPoseEvent, ClientHost, ClientResourcesReloadedEvent, ClientSetupEvent,
    ClientTickPreEvent, ComputeFovEvent, ContainerLootFilledEvent, CustomArmPose,
    ExplosionInteraction, ExplosionSettings, Flow, FluidKey, GameRuleValue, GetArmPoseEvent,
    GuiDraw, GuiLayerPreEvent, GuiRenderPostEvent, Hand, HearsProjectileImpactsEvent, Hooks, Host,
    HostSide, HumanoidPart, IPayloadContext, Identifier, ItemKey, ItemTooltipEvent,
    KeyConflictContext, KeyMappingPreEvent, KeyRegistration, LevelHost, LevelRandomHost, LevelRef,
    LevelTickPostEvent, LightLayer, LivingDamagePreEvent, LivingDeathEvent,
    LivingShieldBlockEvent, LootDropsEvent, ModelBakingCompletedEvent, ParticleKey,
    ParticleOptions, PayloadId, PlayLevelSoundAtPositionEvent, ProjectileImpactEvent, Registrar,
    RegistrationContext, RenderFramePreEvent, RenderHandEvent, RenderPlayerPostEvent, ServerHost,
    ServerTickEvent, ServerTickPhase, SettingChangedEvent, SoundPlay, SoundRef, SoundSource,
    StandaloneModelsEvent, SubmitCustomGeometryEvent, Text, ToClient, ToServer, VarInt,
};
use mod_api::text::TextContent;

use crate::arms::{invalidate_skin_sprite_cache, PlayerArms};
use crate::ballistics::{
    compute_shield_knockback_impulse, compute_shield_rifle_wear, evaluate_ballistics,
    is_ray_headshot_on_box, record_pending_impact, record_pending_shield_block,
    take_pending_impact, take_pending_shield_block, tick_pending_impacts,
};
use crate::constants::*;
use crate::exploration::{
    distance_3d, flush_pending_exploration_advancements, grant_mod_criterion,
    handle_living_death_advancements, is_tracked_rifle_bullet, on_container_loot_filled,
    on_loot_drops, record_bullet_entity_hit, resolve_shooter_and_distance,
    resolve_shot_origin_for_owner,
};
use crate::pose::{
    build_fire_input, cam_recoil_curve, compute_gun_texel_view_pos, draw_back_sling_assembly,
    draw_gun_space_arm, draw_rifle_assembly, evaluate_weapon_pose, local_offhand_rifle_status,
    recoil_envelope, tex_to_unit, ActionAnim, ClientAnimState, RifleModels, SlungRifleState,
};
use crate::server::{
    clear_barrel_heat, consume_bullet_water_splash_on_impact, emit_entity_ballistic_impact_sound,
    find_active_bullet_ray, flush_bullet_flyby_on_impact, on_rifle_input, rounds,
    tick_active_bullets, tick_barrel_heat, tick_reload_sessions, unpack_coord_sub16,
    EntityImpactKind, Winchester,
};
use crate::settings::{
    classify_incoming_level_sound, client_settings, default_channel_for_local_sound,
    mark_settings_dirty, mix_client_sound, refresh_client_settings, refresh_settings_if_dirty,
    register_all_settings, AdsMode, BackSlingStyle, LevelSoundDecision, SoundChannel,
};
use crate::skin::ResolvedSkin;
use crate::world_fx::{
    handle_bullet_terrain_impact, on_bullet_impact_sync, spawn_spent_casing_from_camera,
    spawn_spent_casing_from_remote_actor, step_world_effects, submit_world_effects,
};

// ---------------------------------------------------------------------------
// Network Payloads
// ---------------------------------------------------------------------------

#[derive(horizon_mod_api_macros::PayloadCodec)]
pub(crate) struct RifleInput {
    pub(crate) action: VarInt,
    pub(crate) muzzle_right_mm: VarInt,
    pub(crate) muzzle_up_mm: VarInt,
    pub(crate) muzzle_fwd_mm: VarInt,
    pub(crate) aim_pct: VarInt,
}

impl RifleInput {
    pub(crate) const fn simple(action: i32) -> Self {
        Self {
            action: VarInt(action),
            muzzle_right_mm: VarInt(0),
            muzzle_up_mm: VarInt(0),
            muzzle_fwd_mm: VarInt(0),
            aim_pct: VarInt(0),
        }
    }
}

#[derive(horizon_mod_api_macros::PayloadCodec)]
pub(crate) struct RifleSync {
    pub(crate) kind: VarInt,
    pub(crate) rounds: VarInt,
}

#[derive(horizon_mod_api_macros::PayloadCodec)]
pub(crate) struct RifleSkinSync {
    pub(crate) sprite: String,
    pub(crate) fallback: String,
    pub(crate) slim: VarInt,
}

#[derive(horizon_mod_api_macros::PayloadCodec)]
pub(crate) struct BulletImpactSync {
    pub(crate) dimension: String,
    pub(crate) block_x: VarInt,
    pub(crate) block_y: VarInt,
    pub(crate) block_z: VarInt,
    pub(crate) face_x: VarInt,
    pub(crate) face_y: VarInt,
    pub(crate) face_z: VarInt,
    pub(crate) rel_x_dm: VarInt,
    pub(crate) rel_y_dm: VarInt,
    pub(crate) rel_z_dm: VarInt,
}

#[derive(horizon_mod_api_macros::PayloadCodec)]
pub(crate) struct RifleThirdPersonSync {
    pub(crate) entity_id: VarInt,
    pub(crate) state: VarInt,
}

#[derive(horizon_mod_api_macros::PayloadCodec)]
pub(crate) struct RifleAcousticSync {
    pub(crate) cue: VarInt,
    pub(crate) x_sub16: VarInt,
    pub(crate) y_sub16: VarInt,
    pub(crate) z_sub16: VarInt,
    pub(crate) intensity_pm: VarInt,
}

#[derive(Clone, Copy)]
struct RemoteTpState {
    state: i32,
    set_tick: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ActionSoundTag {
    Draw,
    Inspect,
    Reload,
}

// ---------------------------------------------------------------------------
// Shared Client & Server Runtime State
// ---------------------------------------------------------------------------

pub(crate) static AIMING: AtomicBool = AtomicBool::new(false);
pub(crate) static CLIENT_RELOADING: AtomicBool = AtomicBool::new(false);
pub(crate) static CLIENT_SPRINTING: AtomicBool = AtomicBool::new(false);
pub(crate) static CLIENT_SNEAKING: AtomicBool = AtomicBool::new(false);
static PENDING_FIRE: AtomicBool = AtomicBool::new(false);
static PENDING_RELOAD_CANCEL: AtomicBool = AtomicBool::new(false);
static PENDING_AUTO_RELOAD: AtomicBool = AtomicBool::new(false);
static TOGGLE_AIM_LATCH: AtomicBool = AtomicBool::new(false);
static HEADSHOT_HITMARKER: AtomicBool = AtomicBool::new(false);

pub(crate) static CLIENT_TICKS: AtomicU64 = AtomicU64::new(0);
pub(crate) static AIM_LAST_TICK: AtomicU64 = AtomicU64::new(0);
static LAST_AUTO_RELOAD_TICK: AtomicU64 = AtomicU64::new(0);
static FRAME_SEQ: AtomicU64 = AtomicU64::new(0);
static RIFLE_SEEN_FRAME: AtomicU64 = AtomicU64::new(0);
static ADS_BITS: AtomicU32 = AtomicU32::new(0);
static HITMARKER_TIMER_BITS: AtomicU32 = AtomicU32::new(0);
/// 20 Hz client cache of local player entity ID (`0` when unavailable) and `.44-40` reserve
/// (`u32::MAX` = Creative `∞`).
static LOCAL_ENTITY_ID: AtomicU32 = AtomicU32::new(0);
static LOCAL_RESERVE_AMMO: AtomicU32 = AtomicU32::new(0);
/// Packed 20 Hz cache of `local_offhand_rifle_status`: `0` = not slung, else
/// `1 | ((chest_armored as u32) << 1) | (loaded_rounds << 2)` so `render_player_post` never
/// queries entities/inventories across WASM on every `F5` or Inventory paperdoll frame.
static LOCAL_OFFHAND_PACKED: AtomicU32 = AtomicU32::new(0);
pub(crate) static LAST_WORLD_FOV_BITS: AtomicU32 = AtomicU32::new(0x428C_0000); // 70.0_f32.to_bits()
pub(crate) static LAST_MUZZLE_VIEW_X: AtomicU32 = AtomicU32::new(0x3F07_1A9F); // 0.52775_f32
pub(crate) static LAST_MUZZLE_VIEW_Y: AtomicU32 = AtomicU32::new(0xBEAA_0000); // -0.33203_f32
pub(crate) static LAST_MUZZLE_VIEW_Z: AtomicU32 = AtomicU32::new(0xC001_47AE); // -2.02000_f32
pub(crate) static LAST_EJECTOR_VIEW_X: AtomicU32 = AtomicU32::new(0x3F09_AE14); // 0.5378_f32
pub(crate) static LAST_EJECTOR_VIEW_Y: AtomicU32 = AtomicU32::new(0xBE94_FDF4); // -0.2910_f32
pub(crate) static LAST_EJECTOR_VIEW_Z: AtomicU32 = AtomicU32::new(0xBF80_B852); // -1.0056_f32

pub(crate) static SERVER_TICKS: AtomicU64 = AtomicU64::new(0);

static RIFLE_INPUT: OnceLock<ToServer<RifleInput>> = OnceLock::new();
pub(crate) static RIFLE_SYNC: OnceLock<ToClient<RifleSync>> = OnceLock::new();
pub(crate) static RIFLE_SKIN_SYNC: OnceLock<ToClient<RifleSkinSync>> = OnceLock::new();
pub(crate) static BULLET_IMPACT_SYNC: OnceLock<ToClient<BulletImpactSync>> = OnceLock::new();
pub(crate) static RIFLE_TP_SYNC: OnceLock<ToClient<RifleThirdPersonSync>> = OnceLock::new();
pub(crate) static RIFLE_ACOUSTIC_SYNC: OnceLock<ToClient<RifleAcousticSync>> = OnceLock::new();
static POSE_HIP_READY: OnceLock<CustomArmPose> = OnceLock::new();
static POSE_SPRINT_CARRY: OnceLock<CustomArmPose> = OnceLock::new();
static POSE_LEVER_CYCLE: OnceLock<CustomArmPose> = OnceLock::new();
static POSE_RELOAD: OnceLock<CustomArmPose> = OnceLock::new();
pub(crate) static RIFLE_KEY: OnceLock<ItemKey> = OnceLock::new();
pub(crate) static CARTRIDGE_KEY: OnceLock<ItemKey> = OnceLock::new();
pub(crate) static SMOKE_KEY: OnceLock<ParticleKey> = OnceLock::new();
pub(crate) static WHITE_SMOKE_KEY: OnceLock<ParticleKey> = OnceLock::new();
pub(crate) static FLAME_KEY: OnceLock<ParticleKey> = OnceLock::new();
pub(crate) static POOF_KEY: OnceLock<ParticleKey> = OnceLock::new();
pub(crate) static CRIT_KEY: OnceLock<ParticleKey> = OnceLock::new();
pub(crate) static BLOCK_PARTICLE_KEY: OnceLock<ParticleKey> = OnceLock::new();
pub(crate) static BUBBLE_KEY: OnceLock<ParticleKey> = OnceLock::new();
pub(crate) static BUBBLE_POP_KEY: OnceLock<ParticleKey> = OnceLock::new();
pub(crate) static SPLASH_KEY: OnceLock<ParticleKey> = OnceLock::new();
pub(crate) static WATER_FLUID_KEY: OnceLock<FluidKey> = OnceLock::new();
pub(crate) static FLOWING_WATER_FLUID_KEY: OnceLock<FluidKey> = OnceLock::new();
pub(crate) static BELL_BLOCK_KEY: OnceLock<BlockKey> = OnceLock::new();
pub(crate) static TARGET_BLOCK_KEY: OnceLock<BlockKey> = OnceLock::new();
pub(crate) static DECORATED_POT_BLOCK_KEY: OnceLock<BlockKey> = OnceLock::new();
pub(crate) static CHORUS_FLOWER_BLOCK_KEY: OnceLock<BlockKey> = OnceLock::new();
pub(crate) static POINTED_DRIPSTONE_BLOCK_KEY: OnceLock<BlockKey> = OnceLock::new();
static PLAYER_ARMS: OnceLock<PlayerArms> = OnceLock::new();
pub(crate) static RIFLE_MODELS: Mutex<Option<RifleModels>> = Mutex::new(None);
static CLIENT_ANIM: OnceLock<Mutex<ClientAnimState>> = OnceLock::new();
static SLUNG_RIFLES: Mutex<BTreeMap<u32, SlungRifleState>> = Mutex::new(BTreeMap::new());
static REMOTE_TP_STATES: Mutex<BTreeMap<u32, RemoteTpState>> = Mutex::new(BTreeMap::new());

pub(crate) fn is_water_fluid(fluid: FluidKey) -> bool {
    WATER_FLUID_KEY.get().copied() == Some(fluid)
        || FLOWING_WATER_FLUID_KEY.get().copied() == Some(fluid)
}

/// Checks whether an exact 3D world-space coordinate `[x, y, z]` lies below the water surface
/// in `level` (`Side: both`, handling source water, flowing water, and waterlogged blocks).
pub(crate) fn is_point_in_water(host: &dyn Host, level: LevelRef, pos: [f64; 3]) -> bool {
    let bp = [
        pos[0].floor() as i32,
        pos[1].floor() as i32,
        pos[2].floor() as i32,
    ];
    let Some(fs) = level.fluid_state(host, bp) else {
        return false;
    };
    if fs.is_empty() || !is_water_fluid(fs.fluid) {
        return false;
    }
    let height = level
        .fluid_height_for_camera(host, bp)
        .unwrap_or(f32::from(fs.amount) / 9.0)
        .clamp(0.0, 1.0);
    height > 0.0 && (pos[1] - f64::from(bp[1])) <= f64::from(height)
}

pub(crate) fn client_anim() -> &'static Mutex<ClientAnimState> {
    CLIENT_ANIM.get_or_init(|| Mutex::new(ClientAnimState::new()))
}

pub(crate) fn aim_progress() -> f32 {
    f32::from_bits(ADS_BITS.load(Ordering::Relaxed)).clamp(0.0, 1.2)
}

pub(crate) fn holding_rifle_recently() -> bool {
    let seen = RIFLE_SEEN_FRAME.load(Ordering::Relaxed);
    let now = FRAME_SEQ.load(Ordering::Relaxed);
    seen > 0 && now.saturating_sub(seen) <= 2
}

// ---------------------------------------------------------------------------
// Standalone Model Resolution & Client Sound/Sync Helpers
// ---------------------------------------------------------------------------

fn resolve_rifle_models(host: &dyn ClientHost) -> Option<RifleModels> {
    let assets = host.render_assets();
    let body = assets.model(&Identifier::parse(MODEL_BODY_ID).ok()?).ok()?;
    let body_lod = assets
        .model(&Identifier::parse(MODEL_BODY_LOD_ID).ok()?)
        .unwrap_or(body);
    Some(RifleModels {
        body,
        body_lod,
        lever: assets
            .model(&Identifier::parse(MODEL_LEVER_ID).ok()?)
            .ok()?,
        hammer: assets
            .model(&Identifier::parse(MODEL_HAMMER_ID).ok()?)
            .ok()?,
        trigger: assets
            .model(&Identifier::parse(MODEL_TRIGGER_ID).ok()?)
            .ok()?,
        gate: assets.model(&Identifier::parse(MODEL_GATE_ID).ok()?).ok()?,
        cartridge: assets
            .model(&Identifier::parse(MODEL_CARTRIDGE_ID).ok()?)
            .ok()?,
        casing: assets
            .model(&Identifier::parse(MODEL_CASING_ID).ok()?)
            .ok()?,
        bullet_hole: assets
            .model(&Identifier::parse(MODEL_BULLET_HOLE_ID).ok()?)
            .ok()?,
    })
}

fn refresh_rifle_models(host: &dyn ClientHost) {
    *RIFLE_MODELS.lock().unwrap() = resolve_rifle_models(host);
}

static ACTIVE_ACTION_SOUND: Mutex<Option<(ActionSoundTag, SoundRef)>> = Mutex::new(None);
static REMOTE_ACTION_SOUNDS: Mutex<BTreeMap<u32, SoundRef>> = Mutex::new(BTreeMap::new());
static PENDING_CONCUSSION: Mutex<Option<(f32, f32, u32)>> = Mutex::new(None);
static CONCUSSION_ACTIVE: AtomicBool = AtomicBool::new(false);

const CONCUSSION_EXEMPT_SOUNDS: &[&str] = &[
    SND_FIRE,
    SND_THUMP,
    SND_ECHO_CAVE,
    SND_ECHO_OPEN,
    SND_LEVER,
    SND_COCK,
    SND_BULLET_CRACK,
    SND_BULLET_WHIZ,
];

fn stop_or_fade_sound(host: &dyn ClientHost, sound: SoundRef, fade_ticks: u32) {
    if fade_ticks == 0 {
        let _ = sound.stop(host);
    } else {
        let _ = sound.fade(host, 0.0, fade_ticks);
    }
}

fn cancel_action_sound(host: &dyn ClientHost, filter: Option<ActionSoundTag>, fade_ticks: u32) {
    let prev = {
        let mut slot = ACTIVE_ACTION_SOUND.lock().unwrap();
        match *slot {
            Some((tag, sref)) if filter.is_none_or(|want| want == tag) => {
                *slot = None;
                Some(sref)
            }
            _ => None,
        }
    };
    if let Some(sref) = prev {
        stop_or_fade_sound(host, sref, fade_ticks);
    }
}

fn clear_all_tracked_sounds(host: &dyn ClientHost) {
    cancel_action_sound(host, None, 0);
    let remotes: Vec<SoundRef> = core::mem::take(&mut *REMOTE_ACTION_SOUNDS.lock().unwrap())
        .into_values()
        .collect();
    for sref in remotes {
        let _ = sref.stop(host);
    }
}

fn client_pitch_jitter(salt: u64) -> f32 {
    let mut h = CLIENT_TICKS
        .load(Ordering::Relaxed)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(salt.wrapping_mul(0xBF58_476D_1CE4_E5B9));
    h ^= h >> 30;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 27;
    (((h & 0xFFFF) as f32) * (1.0 / 32_767.5) - 1.0).clamp(-1.0, 1.0)
}

pub(crate) fn play_local_foley_channel(
    host: &dyn ClientHost,
    channel: SoundChannel,
    event_id: &str,
    volume: f32,
    pitch: f32,
) -> Option<SoundRef> {
    let cfg = client_settings();
    let (vol, pit) = mix_client_sound(&cfg, channel, volume, pitch)?;
    let mut sound = SoundPlay::ui(event_id).with_volume(vol).with_pitch(pit);
    sound.source = SoundSource::Players;
    host.play_local_sound(&sound, 0)
}

pub(crate) fn play_local_foley(
    host: &dyn ClientHost,
    event_id: &str,
    volume: f32,
    pitch: f32,
) -> Option<SoundRef> {
    play_local_foley_channel(
        host,
        default_channel_for_local_sound(event_id),
        event_id,
        volume,
        pitch,
    )
}

fn play_tracked_action_sound(
    host: &dyn ClientHost,
    tag: ActionSoundTag,
    channel: SoundChannel,
    event_id: &str,
    volume: f32,
    pitch: f32,
) {
    cancel_action_sound(host, None, 2);
    if let Some(sref) = play_local_foley_channel(host, channel, event_id, volume, pitch) {
        *ACTIVE_ACTION_SOUND.lock().unwrap() = Some((tag, sref));
    }
}

pub(crate) fn play_local_positional_channel(
    host: &dyn ClientHost,
    channel: SoundChannel,
    event_id: &str,
    position: [f64; 3],
    volume: f32,
    pitch: f32,
    after_ticks: u32,
) -> Option<SoundRef> {
    let cfg = client_settings();
    let (vol, pit) = mix_client_sound(&cfg, channel, volume, pitch)?;
    let sound = SoundPlay::at(event_id, SoundSource::Players, position)
        .with_volume(vol)
        .with_pitch(pit);
    host.play_local_sound(&sound, after_ticks)
}

pub(crate) fn play_local_positional_foley(
    host: &dyn ClientHost,
    event_id: &str,
    position: [f64; 3],
    volume: f32,
    pitch: f32,
    after_ticks: u32,
) -> Option<SoundRef> {
    play_local_positional_channel(
        host,
        default_channel_for_local_sound(event_id),
        event_id,
        position,
        volume,
        pitch,
        after_ticks,
    )
}

fn on_rifle_sync(value: RifleSync, cx: &mut dyn IPayloadContext) -> Result<(), String> {
    let cfg = client_settings();
    if value.kind.0 == SYNC_HIT_CONFIRM || value.kind.0 == SYNC_HEADSHOT_CONFIRM {
        let is_headshot = value.kind.0 == SYNC_HEADSHOT_CONFIRM;
        HEADSHOT_HITMARKER.store(is_headshot, Ordering::Relaxed);
        HITMARKER_TIMER_BITS.store(
            (if is_headshot { 0.22_f32 } else { 0.14_f32 }).to_bits(),
            Ordering::Relaxed,
        );
        if let Some((snd, vol, pitch)) = cfg.hit_confirm_sound.sound_spec()
            && let Ok(host) = cx.client_host()
        {
            if is_headshot {
                let _ = play_local_foley_channel(
                    host,
                    SoundChannel::Feedback,
                    snd,
                    (vol * 1.18).min(1.0),
                    (pitch * 1.18).min(2.0),
                );
                let _ = play_local_foley_channel(
                    host,
                    SoundChannel::Feedback,
                    SND_HIT_CONFIRM_CHIME,
                    (vol * 0.85).min(0.85),
                    1.82,
                );
            } else {
                let _ = play_local_foley_channel(host, SoundChannel::Feedback, snd, vol, pitch);
            }
        }
        return Ok(());
    }

    let rounds_now = (value.rounds.0.max(0) as u32).min(TUBE_CAPACITY);
    let mut tracked_foley: Option<(ActionSoundTag, SoundChannel, &'static str, f32, f32)> = None;
    let mut oneshot_foley: Option<(SoundChannel, &'static str, f32, f32)> = None;
    let mut stop_tracked = false;

    {
        let mut anim = client_anim().lock().unwrap();
        anim.known_rounds = rounds_now;
        let j = client_pitch_jitter(u64::from(rounds_now) + (value.kind.0 as u64) * 31);
        match value.kind.0 {
            SYNC_SHOT => {
                CLIENT_RELOADING.store(false, Ordering::Relaxed);
                stop_tracked = true;
                anim.on_shot(rounds_now);
            }
            SYNC_DRY_CLICK => {
                stop_tracked = true;
                anim.on_dry_click(rounds_now);
                if cfg.dry_click_alert {
                    oneshot_foley = Some((SoundChannel::Feedback, SND_DRY_FIRE, 0.74, 1.0 + 0.03 * j));
                }
                if rounds_now == 0 && cfg.auto_reload {
                    PENDING_AUTO_RELOAD.store(true, Ordering::Relaxed);
                }
            }
            SYNC_WET_MISFIRE => {
                // World-space Mono `SND_MISFIRE_WET` is already emitted at the flooded breech on
                // the server (`emit_wet_misfire`); only stop interrupted foley & animate trigger here.
                CLIENT_RELOADING.store(false, Ordering::Relaxed);
                stop_tracked = true;
                anim.on_dry_click(rounds_now);
            }
            SYNC_RELOAD_START => {
                CLIENT_RELOADING.store(true, Ordering::Relaxed);
                AIMING.store(false, Ordering::Relaxed);
                TOGGLE_AIM_LATCH.store(false, Ordering::Relaxed);
                anim.action = ActionAnim::ReloadIntro { elapsed: 0.0 };
                tracked_foley = Some((
                    ActionSoundTag::Reload,
                    SoundChannel::Reload,
                    SND_RELOAD_INTRO,
                    0.90,
                    1.0 + 0.025 * j,
                ));
            }
            SYNC_SHELL_INSERTED => {
                CLIENT_RELOADING.store(true, Ordering::Relaxed);
                anim.action = ActionAnim::ReloadInsert { elapsed: 0.0 };
                tracked_foley = Some((
                    ActionSoundTag::Reload,
                    SoundChannel::Reload,
                    SND_RELOAD_INSERT,
                    0.92,
                    1.0 + 0.03 * j,
                ));
            }
            SYNC_RELOAD_END_NORM => {
                CLIENT_RELOADING.store(false, Ordering::Relaxed);
                anim.action = ActionAnim::ReloadEndNorm { elapsed: 0.0 };
                tracked_foley = Some((
                    ActionSoundTag::Reload,
                    SoundChannel::Reload,
                    SND_RELOAD_END,
                    0.88,
                    1.0 + 0.02 * j,
                ));
            }
            SYNC_RELOAD_END_EMPTY => {
                // Empty-chamber close cycles the lever at `t = 0.15s` + cocks the hammer at `t = 0.40s`
                // (`Mechanism` channel); fade any tail from the last shell insert instead of overlapping
                // `SND_RELOAD_END` on top of the lever throw.
                CLIENT_RELOADING.store(false, Ordering::Relaxed);
                stop_tracked = true;
                anim.action = ActionAnim::ReloadEndEmpty {
                    elapsed: 0.0,
                    lever_sound_played: false,
                    cock_sound_played: false,
                };
            }
            SYNC_RELOAD_ABORTED => {
                CLIENT_RELOADING.store(false, Ordering::Relaxed);
                stop_tracked = true;
                if matches!(
                    anim.action,
                    ActionAnim::ReloadIntro { .. }
                        | ActionAnim::ReloadInsert { .. }
                        | ActionAnim::ReloadEndNorm { .. }
                        | ActionAnim::ReloadEndEmpty { .. }
                ) {
                    anim.action = ActionAnim::None;
                }
            }
            _ => {}
        }
    }

    if let Ok(host) = cx.client_host() {
        if stop_tracked {
            cancel_action_sound(host, None, 2);
        }
        if let Some((tag, ch, snd, vol, pitch)) = tracked_foley {
            play_tracked_action_sound(host, tag, ch, snd, vol, pitch);
        }
        if let Some((ch, snd, vol, pitch)) = oneshot_foley {
            let _ = play_local_foley_channel(host, ch, snd, vol, pitch);
        }
    }
    Ok(())
}

fn on_rifle_skin_sync(value: RifleSkinSync, _: &mut dyn IPayloadContext) -> Result<(), String> {
    invalidate_skin_sprite_cache();
    let mut anim = client_anim().lock().unwrap();
    anim.skin = ResolvedSkin {
        primary: value.sprite,
        fallback: value.fallback,
        slim: value.slim.0 != 0,
    };
    Ok(())
}

fn resolve_remote_breech_pos(host: &dyn ClientHost, eid: u32) -> Option<[f64; 3]> {
    let ent = host.entity(eid)?;
    if let Some(eye) = ent.eye_position(host) {
        Some([eye[0], eye[1] - 0.24, eye[2]])
    } else {
        ent.position(host).map(|p| [p[0], p[1] + 1.32, p[2]])
    }
}

fn replace_remote_action_sound(host: &dyn ClientHost, eid: u32, next: Option<SoundRef>) {
    let prev = {
        let mut map = REMOTE_ACTION_SOUNDS.lock().unwrap();
        if map.len() >= 64 && !map.contains_key(&eid) {
            map.clear();
        }
        match next {
            Some(sref) => map.insert(eid, sref),
            None => map.remove(&eid),
        }
    };
    if let Some(old) = prev {
        stop_or_fade_sound(host, old, 2);
    }
}

fn on_rifle_tp_sync(
    value: RifleThirdPersonSync,
    cx: &mut dyn IPayloadContext,
) -> Result<(), String> {
    if value.entity_id.0 <= 0 {
        return Ok(());
    }
    let eid = value.entity_id.0 as u32;
    let now = CLIENT_TICKS.load(Ordering::Relaxed);
    let cue = value.state.0;

    // 1. Update 3rd-person remote pose table (`TP_CUE_DRY_CLICK` is audio-only, preserving pose).
    if cue != TP_CUE_DRY_CLICK {
        let mut map = REMOTE_TP_STATES.lock().unwrap();
        match cue {
            TP_STATE_IDLE | TP_CUE_RELOAD_END_NORM | TP_CUE_LAST_SHOT_EJECT => {
                map.remove(&eid);
            }
            TP_STATE_RELOADING | TP_CUE_RELOAD_INSERT => {
                if map.len() >= 64 {
                    map.retain(|_, v| now.saturating_sub(v.set_tick) <= 120);
                }
                map.insert(
                    eid,
                    RemoteTpState {
                        state: TP_STATE_RELOADING,
                        set_tick: now,
                    },
                );
            }
            TP_STATE_LEVER_CYCLE | TP_CUE_EMPTY_CHAMBER_LEVER => {
                if map.len() >= 64 {
                    map.retain(|_, v| now.saturating_sub(v.set_tick) <= 120);
                }
                map.insert(
                    eid,
                    RemoteTpState {
                        state: TP_STATE_LEVER_CYCLE,
                        set_tick: now,
                    },
                );
            }
            _ => {}
        }
    }

    // 2. Emit 3D Mono positional foley & spent brass casings for nearby remote players (`eid != local_eid`).
    if eid == LOCAL_ENTITY_ID.load(Ordering::Relaxed) {
        return Ok(());
    }
    let Ok(host) = cx.client_host() else {
        return Ok(());
    };
    if cue == TP_STATE_IDLE {
        replace_remote_action_sound(host, eid, None);
        return Ok(());
    }
    let Some(pos) = resolve_remote_breech_pos(host, eid) else {
        return Ok(());
    };
    let j = client_pitch_jitter(u64::from(eid).wrapping_mul(17) ^ (cue as u64));
    match cue {
        TP_STATE_RELOADING => {
            let sref = play_local_positional_channel(
                host,
                SoundChannel::Reload,
                SND_RELOAD_INTRO,
                pos,
                0.78,
                1.0 + 0.025 * j,
                0,
            );
            replace_remote_action_sound(host, eid, sref);
        }
        TP_CUE_RELOAD_INSERT => {
            let sref = play_local_positional_channel(
                host,
                SoundChannel::Reload,
                SND_RELOAD_INSERT,
                pos,
                0.82,
                1.0 + 0.03 * j,
                0,
            );
            replace_remote_action_sound(host, eid, sref);
        }
        TP_CUE_RELOAD_END_NORM => {
            let sref = play_local_positional_channel(
                host,
                SoundChannel::Reload,
                SND_RELOAD_END,
                pos,
                0.76,
                1.0 + 0.02 * j,
                0,
            );
            replace_remote_action_sound(host, eid, sref);
        }
        TP_STATE_LEVER_CYCLE => {
            replace_remote_action_sound(host, eid, None);
            spawn_spent_casing_from_remote_actor(host, eid);
            let _ = play_local_positional_channel(
                host,
                SoundChannel::Mechanism,
                SND_LEVER,
                pos,
                0.80,
                1.0 + 0.025 * j,
                3,
            );
            let _ = play_local_positional_channel(
                host,
                SoundChannel::Mechanism,
                SND_COCK,
                pos,
                0.66,
                1.02 + 0.025 * j,
                8,
            );
        }
        TP_CUE_LAST_SHOT_EJECT => {
            replace_remote_action_sound(host, eid, None);
            spawn_spent_casing_from_remote_actor(host, eid);
        }
        TP_CUE_EMPTY_CHAMBER_LEVER => {
            replace_remote_action_sound(host, eid, None);
            let _ = play_local_positional_channel(
                host,
                SoundChannel::Mechanism,
                SND_LEVER,
                pos,
                0.80,
                1.0 + 0.025 * j,
                3,
            );
            let _ = play_local_positional_channel(
                host,
                SoundChannel::Mechanism,
                SND_COCK,
                pos,
                0.66,
                1.02 + 0.025 * j,
                8,
            );
        }
        TP_CUE_DRY_CLICK => {
            let _ = play_local_positional_channel(
                host,
                SoundChannel::Mechanism,
                SND_DRY_FIRE,
                pos,
                0.72,
                1.0 + 0.03 * j,
                0,
            );
        }
        _ => {}
    }
    Ok(())
}

fn on_rifle_acoustic_sync(
    value: RifleAcousticSync,
    cx: &mut dyn IPayloadContext,
) -> Result<(), String> {
    let Ok(host) = cx.client_host() else {
        return Ok(());
    };
    let cfg = client_settings();
    let pos = [
        unpack_coord_sub16(value.x_sub16),
        unpack_coord_sub16(value.y_sub16),
        unpack_coord_sub16(value.z_sub16),
    ];
    let intensity = (value.intensity_pm.0 as f32 * 0.001).clamp(0.0, 1.0);

    let cam = host.camera();
    let listener_pos = cam
        .as_ref()
        .map(|c| c.position)
        .or_else(|| host.local_player().and_then(|p| p.entity().eye_position(host)));
    let dist = listener_pos.map_or(0.0, |lp| distance_3d(pos, lp));
    // Speed of sound (~340 m/s = 17 blocks/tick at 20 Hz) for realistic downrange report arrival.
    let prop_ticks = (dist / 17.0).floor().clamp(0.0, 12.0) as u32;
    let salt = (value.x_sub16.0 as u64)
        .wrapping_mul(31)
        .wrapping_add(value.z_sub16.0 as u64);
    let j = client_pitch_jitter(salt);

    match value.cue.0 {
        ACOUSTIC_CUE_BULLET_CRACK => {
            let vol = 0.72 + 0.28 * intensity;
            let pitch = 0.96 + 0.12 * intensity + 0.03 * j;
            let _ = play_local_positional_channel(
                host,
                SoundChannel::WorldImpact,
                SND_BULLET_CRACK,
                pos,
                vol,
                pitch,
                0,
            );
            // Very close supersonic graze (`<= ~3m`) trails a subtle aerodynamic slipstream wake.
            if intensity >= 0.45 {
                let _ = play_local_positional_channel(
                    host,
                    SoundChannel::WorldImpact,
                    SND_BULLET_WHIZ,
                    pos,
                    0.36 * intensity,
                    1.06 + 0.03 * j,
                    1,
                );
            }
        }
        ACOUSTIC_CUE_BULLET_WHIZ => {
            let vol = 0.42 + 0.50 * intensity;
            let pitch = 0.90 + 0.18 * intensity + 0.03 * j;
            let _ = play_local_positional_channel(
                host,
                SoundChannel::WorldImpact,
                SND_BULLET_WHIZ,
                pos,
                vol,
                pitch,
                0,
            );
        }
        ACOUSTIC_CUE_SHOT_CONFINED => {
            let (vol, pitch) = if dist <= 32.0 {
                (
                    0.62 + 0.42 * intensity,
                    0.96 + 0.06 * (1.0 - intensity) + 0.025 * j,
                )
            } else {
                let fade = (1.0 - ((dist - 32.0) / 68.0) as f32).clamp(0.18, 1.0);
                (1.55 * fade, 0.88 + 0.025 * j)
            };
            let _ = play_local_positional_channel(
                host,
                SoundChannel::Echo,
                SND_ECHO_CAVE,
                pos,
                vol,
                pitch,
                (1 + prop_ticks).min(14),
            );

            // Subtle indoor/cave blast pressure concussion when detonating right near the listener.
            if cfg.blast_concussion
                && cfg.vol_master > 0.01
                && cfg.vol_fire > 0.01
                && dist <= 5.0
                && let Some(lp) = listener_pos
            {
                let dim = cam
                    .as_ref()
                    .map_or(DEFAULT_DIMENSION, |c| c.dimension.as_str());
                let lbp = [lp[0].floor() as i32, lp[1].floor() as i32, lp[2].floor() as i32];
                let listener_sky = host
                    .level(dim)
                    .and_then(|lvl| lvl.brightness(host, LightLayer::Sky, lbp))
                    .unwrap_or(0);
                if listener_sky <= 9 {
                    let prox = (1.0 - (dist as f32 / 5.0)).clamp(0.0, 1.0);
                    let amount = ((0.14 + 0.18 * intensity) * prox).clamp(0.05, 0.34);
                    let lowpass = ((0.28 + 0.26 * intensity) * prox).clamp(0.10, 0.56);
                    let ticks = (10.0 + 10.0 * intensity * prox).round().clamp(8.0, 20.0) as u32;
                    *PENDING_CONCUSSION.lock().unwrap() = Some((amount, lowpass, ticks));
                }
            }
        }
        ACOUSTIC_CUE_SHOT_SEMI => {
            let _ = play_local_positional_channel(
                host,
                SoundChannel::Echo,
                SND_ECHO_CAVE,
                pos,
                0.30 * intensity,
                1.07 + 0.02 * j,
                (1 + prop_ticks).min(14),
            );
            let _ = play_local_positional_channel(
                host,
                SoundChannel::Echo,
                SND_ECHO_OPEN,
                pos,
                0.46 * intensity,
                0.98 + 0.02 * j,
                (3 + prop_ticks).min(16),
            );
        }
        ACOUSTIC_CUE_SHOT_OPEN => {
            let (vol, pitch, after) = if dist <= 32.0 {
                let base_delay = if intensity >= 0.72 { 2 } else { 3 };
                (
                    0.40 + 0.50 * intensity,
                    0.98 + 0.025 * j,
                    base_delay + prop_ticks,
                )
            } else {
                // Distant listeners (>32..96m) hear the rolling valley report arrive at speed of sound.
                let fade = (1.0 - ((dist - 32.0) / 68.0) as f32).clamp(0.16, 1.0);
                (
                    (1.55 + 0.45 * intensity) * fade,
                    0.91 + 0.025 * j,
                    prop_ticks.clamp(2, 12),
                )
            };
            let _ = play_local_positional_channel(
                host,
                SoundChannel::Echo,
                SND_ECHO_OPEN,
                pos,
                vol,
                pitch,
                after,
            );
        }
        _ => {}
    }
    Ok(())
}

fn count_local_reserve_ammo(host: &dyn ClientHost) -> u32 {
    let Some(cartridge) = CARTRIDGE_KEY.get().copied() else {
        return 0;
    };
    let Some(player) = host.local_player() else {
        return 0;
    };
    if player.is_creative(host).unwrap_or(false) {
        return u32::MAX;
    }
    let mut total: u32 = 0;
    if let Some(items) = player.inventory(host) {
        for st in items {
            if st.item() == cartridge {
                total = total.saturating_add(st.count());
            }
        }
    }
    if let Some(liv) = host.living(player.entity())
        && let Some(off) = liv.hand_item(host, Hand::Off)
        && off.item() == cartridge
    {
        total = total.saturating_add(off.count());
    }
    total
}

fn styled_text(mut text: Text, color_hex: &str, italic: bool) -> Text {
    text.style.color = Some(color_hex.into());
    text.style.italic = Some(italic);
    text
}

fn pretty_key_fallback(key_id: &str, default_label: &str) -> String {
    if let Some(rest) = key_id.strip_prefix("key.keyboard.") {
        if rest.len() == 1 {
            return rest.to_ascii_uppercase();
        }
        return rest.replace('.', " ").to_ascii_uppercase();
    }
    if let Some(rest) = key_id.strip_prefix("key.mouse.") {
        return format!("M:{}", rest.to_ascii_uppercase());
    }
    default_label.into()
}

fn mod_key_component(host: &dyn ClientHost, mapping: &str, fallback_label: &str) -> Text {
    let base = match host.mod_key(MOD_ID, mapping) {
        Some((key_id, _)) if !key_id.is_empty() && key_id != "key.keyboard.unknown" => {
            Text::new(TextContent::Translatable {
                key: key_id.clone(),
                fallback: Some(pretty_key_fallback(&key_id, fallback_label)),
                arguments: Vec::new(),
            })
        }
        _ => Text::literal(fallback_label),
    };
    styled_text(base, "#F2D56F", false)
}

fn vanilla_key_component(mapping: &str) -> Text {
    styled_text(
        Text::new(TextContent::Keybind(mapping.into())),
        "#F2D56F",
        false,
    )
}

fn text_contains_durability(text: &Text) -> bool {
    if let TextContent::Translatable { key, .. } = &text.content
        && key == "item.durability"
    {
        return true;
    }
    text.children.iter().any(text_contains_durability)
}

/// Draws a crisp pixel-perfect mini `.44-40` brass/copper cartridge icon (`3×9` GUI px) inside
/// the HUD reserve counter box with zero texture state switches.
fn draw_hud_mini_cartridge(event: &mut GuiRenderPostEvent<'_>, x: f32, y: f32) {
    // Copper jacketed soft-lead nose (top 3px)
    event.batch.fill([x + 0.5, y, 2.0, 1.0], 0xFF_F2_AD_72);
    event.batch.fill([x, y + 1.0, 3.0, 2.0], 0xFF_DB_89_48);
    event.batch.fill([x, y + 1.0, 1.0, 2.0], 0xFF_F2_AD_72);
    // Polished .44-40 bottleneck brass case (middle 5px)
    event.batch.fill([x, y + 3.0, 3.0, 5.0], 0xFF_BF_95_28);
    event.batch.fill([x + 0.5, y + 3.0, 1.2, 5.0], 0xFF_F2_D5_6F);
    // Rimmed centerfire base (bottom 1px, slightly wider)
    event.batch.fill([x - 0.5, y + 8.0, 4.0, 1.0], 0xFF_99_73_1C);
    event.batch.fill([x, y + 8.0, 2.0, 1.0], 0xFF_E6_C0_4C);
}

// ---------------------------------------------------------------------------
// Entry Point & Hooks
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn standalone_models(&self, _: &RegistrationContext, event: &mut StandaloneModelsEvent<'_>) {
        for id in [
            MODEL_BODY_ID,
            MODEL_BODY_LOD_ID,
            MODEL_LEVER_ID,
            MODEL_HAMMER_ID,
            MODEL_TRIGGER_ID,
            MODEL_GATE_ID,
            MODEL_CARTRIDGE_ID,
            MODEL_CASING_ID,
            MODEL_BULLET_HOLE_ID,
        ] {
            event.models.push(id.into());
        }
    }

    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let winchester = r.item_key(WINCHESTER);
        let cartridge = r.item_key(CARTRIDGE);
        let smoke = r.particle_key(SMOKE_PARTICLE);
        let white_smoke = r.particle_key(WHITE_SMOKE_PARTICLE);
        let flame = r.particle_key(FLAME_PARTICLE);
        let poof = r.particle_key(POOF_PARTICLE);
        let crit = r.particle_key(CRIT_PARTICLE);
        let block_particle = r.particle_key(BLOCK_PARTICLE);
        let bubble = r.particle_key(BUBBLE_PARTICLE);
        let bubble_pop = r.particle_key(BUBBLE_POP_PARTICLE);
        let splash = r.particle_key(SPLASH_PARTICLE);
        let water_fluid = r.fluid_key(WATER_FLUID);
        let flowing_water_fluid = r.fluid_key(FLOWING_WATER_FLUID);
        let bell_block = r.block_key(BELL_BLOCK);
        let target_block = r.block_key(TARGET_BLOCK);
        let decorated_pot_block = r.block_key(DECORATED_POT_BLOCK);
        let chorus_flower_block = r.block_key(CHORUS_FLOWER_BLOCK);
        let pointed_dripstone_block = r.block_key(POINTED_DRIPSTONE_BLOCK);
        r.item_part(winchester, Winchester);
        let _ = RIFLE_KEY.set(winchester);
        let _ = CARTRIDGE_KEY.set(cartridge);
        let _ = SMOKE_KEY.set(smoke);
        let _ = WHITE_SMOKE_KEY.set(white_smoke);
        let _ = FLAME_KEY.set(flame);
        let _ = POOF_KEY.set(poof);
        let _ = CRIT_KEY.set(crit);
        let _ = BLOCK_PARTICLE_KEY.set(block_particle);
        let _ = BUBBLE_KEY.set(bubble);
        let _ = BUBBLE_POP_KEY.set(bubble_pop);
        let _ = SPLASH_KEY.set(splash);
        let _ = WATER_FLUID_KEY.set(water_fluid);
        let _ = FLOWING_WATER_FLUID_KEY.set(flowing_water_fluid);
        let _ = BELL_BLOCK_KEY.set(bell_block);
        let _ = TARGET_BLOCK_KEY.set(target_block);
        let _ = DECORATED_POT_BLOCK_KEY.set(decorated_pot_block);
        let _ = CHORUS_FLOWER_BLOCK_KEY.set(chorus_flower_block);
        let _ = POINTED_DRIPSTONE_BLOCK_KEY.set(pointed_dripstone_block);

        r.register_key_mapping(
            KeyRegistration::new(RELOAD_MAPPING, "key.keyboard.r")
                .label("key.just_a_rifle.reload")
                .context(KeyConflictContext::InGame),
        );
        r.register_key_mapping(
            KeyRegistration::new(INSPECT_MAPPING, "key.keyboard.i")
                .label("key.just_a_rifle.inspect")
                .context(KeyConflictContext::InGame),
        );
        let _ = POSE_HIP_READY.set(r.register_arm_pose(POSE_HIP_READY_ID, true, true));
        let _ = POSE_SPRINT_CARRY.set(r.register_arm_pose(POSE_SPRINT_CARRY_ID, true, true));
        let _ = POSE_LEVER_CYCLE.set(r.register_arm_pose(POSE_LEVER_CYCLE_ID, true, true));
        let _ = POSE_RELOAD.set(r.register_arm_pose(POSE_RELOAD_ID, true, true));
        let _ = register_all_settings(r);

        let mut payloads = r.payload_handlers();
        let mut view = payloads.registrar("1");
        let input = view.play_to_server_typed::<RifleInput>(
            PayloadId(INPUT_CHANNEL.into()),
            on_rifle_input,
        );
        let sync =
            view.play_to_client_typed::<RifleSync>(PayloadId(SYNC_CHANNEL.into()), on_rifle_sync);
        let skin = view.play_to_client_typed::<RifleSkinSync>(
            PayloadId(SKIN_CHANNEL.into()),
            on_rifle_skin_sync,
        );
        let impact = view.play_to_client_typed::<BulletImpactSync>(
            PayloadId(IMPACT_CHANNEL.into()),
            on_bullet_impact_sync,
        );
        let tp_sync = view.play_to_client_typed::<RifleThirdPersonSync>(
            PayloadId(TP_STATE_CHANNEL.into()),
            on_rifle_tp_sync,
        );
        let acoustic_sync = view.play_to_client_typed::<RifleAcousticSync>(
            PayloadId(ACOUSTIC_CHANNEL.into()),
            on_rifle_acoustic_sync,
        );
        let _ = RIFLE_INPUT.set(input);
        let _ = RIFLE_SYNC.set(sync);
        let _ = RIFLE_SKIN_SYNC.set(skin);
        let _ = BULLET_IMPACT_SYNC.set(impact);
        let _ = RIFLE_TP_SYNC.set(tp_sync);
        let _ = RIFLE_ACOUSTIC_SYNC.set(acoustic_sync);
    }

    fn hears_projectile_impacts(&self, _: &dyn Host, event: &mut HearsProjectileImpactsEvent) {
        event.hears = true;
    }

    fn projectile_impact(
        &self,
        host: &mut dyn ServerHost,
        event: &mut ProjectileImpactEvent<'_>,
    ) -> Flow {
        if event.kind == BULLET {
            if event.entity.is_none() {
                handle_bullet_terrain_impact(host, event);
            } else if let Some(victim_id) = event.entity
                && victim_id != event.owner
                && let Some(level) = host.level(event.dimension)
            {
                let now = SERVER_TICKS.load(Ordering::Relaxed);
                consume_bullet_water_splash_on_impact(host, level, event.at);
                flush_bullet_flyby_on_impact(host, level, event.owner, event.at, Some(victim_id));

                let victim_ent = host.entity(victim_id);
                let shooter_dist = if event.owner != 0 {
                    resolve_shooter_and_distance(host, level, event.owner, event.at)
                } else {
                    None
                };

                // Sympathetic `.44-40` powder-keg detonation on `minecraft:tnt_minecart`!
                if event.entity_kind == ENTITY_TNT_MINECART
                    && !is_point_in_water(host, level, event.at)
                    && !matches!(
                        level.game_rule(host, RULE_TNT_EXPLODES),
                        Ok(Some(GameRuleValue::Bool(false)))
                    )
                    && let Some(cart_ent) = victim_ent
                {
                    if let Some(proj) = host.entity(event.projectile) {
                        let _ = proj.discard(host);
                    }
                    let _ = cart_ent.discard(host);
                    let _ = level.explode(
                        host,
                        shooter_dist.as_ref().map(|(s, _)| s.entity()),
                        event.at,
                        4.0,
                        false,
                        ExplosionInteraction::Tnt,
                        ExplosionSettings::default(),
                    );
                    if let Some((shooter, _)) = shooter_dist {
                        grant_mod_criterion(host, shooter, TRIG_POWDER_KEG);
                        if let Some(sync) = RIFLE_SYNC.get() {
                            let _ = sync.send(
                                host,
                                shooter,
                                &RifleSync {
                                    kind: VarInt(SYNC_HIT_CONFIRM),
                                    rounds: VarInt(0),
                                },
                            );
                        }
                    }
                    return Flow::Cancel;
                }

                let aabb = victim_ent.and_then(|e| e.bounding_box(host));
                let eye_h = victim_ent.and_then(|e| e.eye_height(host));

                let (ray_origin, ray_dir) = find_active_bullet_ray(host, event.at).unwrap_or_else(
                    || {
                        let orig = resolve_shot_origin_for_owner(event.owner)
                            .or_else(|| {
                                shooter_dist
                                    .as_ref()
                                    .and_then(|(s, _)| s.entity().eye_position(host))
                            })
                            .unwrap_or(event.at);
                        (
                            orig,
                            [
                                event.at[0] - orig[0],
                                event.at[1] - orig[1],
                                event.at[2] - orig[2],
                            ],
                        )
                    },
                );

                let is_headshot = aabb.is_some_and(|bounds| {
                    is_ray_headshot_on_box(ray_origin, ray_dir, event.at, bounds, eye_h)
                });
                let impact_kind = emit_entity_ballistic_impact_sound(
                    host,
                    level,
                    victim_id,
                    event.at,
                    ray_dir,
                    aabb,
                    is_headshot,
                    now,
                );

                if impact_kind == EntityImpactKind::ShieldBlock {
                    // Heavy `.44-40` soft-lead slug slams the raised shield: apply momentum push,
                    // metallic spark shower, and queue rifle shield-durability wear without a false
                    // headshot hitmarker.
                    if let Some(ent) = victim_ent
                        && let Some(impulse) = compute_shield_knockback_impulse(ray_dir)
                    {
                        let _ = ent.push(host, impulse);
                    }
                    if let Some(crit) = CRIT_KEY.get().copied() {
                        let crit_opts = ParticleOptions::simple(crit);
                        level.send_particles(
                            host,
                            &crit_opts,
                            None,
                            event.at,
                            [0.08, 0.08, 0.08],
                            0.22,
                            6,
                        );
                    }
                    if let Some(white_smoke) = WHITE_SMOKE_KEY.get().copied() {
                        let smoke_opts = ParticleOptions::simple(white_smoke);
                        level.send_particles(
                            host,
                            &smoke_opts,
                            None,
                            event.at,
                            [0.05, 0.05, 0.05],
                            0.018,
                            3,
                        );
                    }
                    let dist = shooter_dist.map_or(0.0, |(_, d)| d);
                    let eval = evaluate_ballistics(dist, false);
                    record_pending_shield_block(
                        event.owner as i32,
                        victim_id as i32,
                        eval.total_multiplier,
                    );
                } else if let Some((shooter, dist)) = shooter_dist {
                    let eval = evaluate_ballistics(dist, is_headshot);
                    record_pending_impact(event.owner as i32, victim_id as i32, eval);
                    record_bullet_entity_hit(victim_id, shooter.uuid(), dist, eval.is_headshot, now);

                    if eval.is_headshot && let Some(crit) = CRIT_KEY.get().copied() {
                        let head_y = aabb.map_or(event.at[1], |b| {
                            (b[0][1] + eye_h.unwrap_or((b[1][1] - b[0][1]) * 0.85)).min(b[1][1])
                        });
                        let crit_opts = ParticleOptions::simple(crit);
                        level.send_particles(
                            host,
                            &crit_opts,
                            None,
                            [event.at[0], head_y, event.at[2]],
                            [0.14, 0.14, 0.14],
                            0.18,
                            7,
                        );
                    }

                    if let Some(sync) = RIFLE_SYNC.get() {
                        let _ = sync.send(
                            host,
                            shooter,
                            &RifleSync {
                                kind: VarInt(if is_headshot {
                                    SYNC_HEADSHOT_CONFIRM
                                } else {
                                    SYNC_HIT_CONFIRM
                                }),
                                rounds: VarInt(0),
                            },
                        );
                    }
                }
            }
        }
        Flow::Continue
    }

    fn living_shield_block(
        &self,
        _level: &mut dyn LevelRandomHost,
        event: &mut LivingShieldBlockEvent<'_>,
    ) {
        if !event.block.blocked || event.source.direct <= 0 {
            return;
        }
        let now = SERVER_TICKS.load(Ordering::Relaxed);
        let bullet_u32 = event.source.direct as u32;
        let mult = if let Some(m) = take_pending_shield_block(event.source.causing, event.entity) {
            Some(m)
        } else if is_tracked_rifle_bullet(bullet_u32, now) {
            Some(1.0)
        } else {
            None
        };
        if let Some(m) = mult {
            let wear = compute_shield_rifle_wear(event.damage * m);
            event.block.shield_damage = event.block.shield_damage.max(wear);
        }
    }

    fn anvil_update(&self, _host: HostSide<'_>, event: &mut AnvilUpdateEvent<'_>) -> Flow {
        let Some(rifle) = RIFLE_KEY.get().copied() else {
            return Flow::Continue;
        };
        if event.update.left.item() != rifle {
            return Flow::Continue;
        }
        if let Some(ref mut out) = event.update.output {
            // Preserve loaded `.44-40` rounds (`damage`) across renames & enchantment books.
            out.set_damage(event.update.left.damage());
            // Reject pointless two-item combinations whose only effect in vanilla was "repairing"
            // durability (`damage -> 0`), so an anvil never eats a second Winchester for nothing.
            if event.update.right.is_some() && *out == event.update.left {
                return Flow::Cancel;
            }
        }
        Flow::Continue
    }

    fn loot_drops(&self, host: &mut dyn ServerHost, event: &mut LootDropsEvent<'_>) {
        let now = SERVER_TICKS.load(Ordering::Relaxed);
        on_loot_drops(host, event, now);
    }

    fn living_damage_pre(
        &self,
        _level: &mut dyn LevelRandomHost,
        event: &mut LivingDamagePreEvent<'_>,
    ) {
        if event.source.direct <= 0 {
            return;
        }
        let now = SERVER_TICKS.load(Ordering::Relaxed);
        let bullet_u32 = event.source.direct as u32;

        // Fast path: consume the exact sub-tick ray-AABB evaluation recorded moments earlier in
        // `projectile_impact` for `(owner = source.causing, target = event.entity)`.
        let eval = if let Some(pending) = take_pending_impact(event.source.causing, event.entity) {
            Some(pending)
        } else if is_tracked_rifle_bullet(bullet_u32, now) {
            // Defensive fallback if `living_damage_pre` ever fires without a prior cached entry.
            let dist = if event.source.causing > 0 {
                let orig = resolve_shot_origin_for_owner(event.source.causing as u32);
                match (orig, event.source.position) {
                    (Some(a), Some(b)) => distance_3d(a, b),
                    _ => 0.0,
                }
            } else {
                0.0
            };
            Some(evaluate_ballistics(dist, false))
        } else {
            None
        };

        if let Some(ev) = eval {
            let base = event.container.new_damage();
            if base > 0.0 && (ev.total_multiplier - 1.0).abs() > 1e-4 {
                event.container.set_new_damage(base * ev.total_multiplier);
            }
        }
    }

    fn item_tooltip(&self, host: &dyn ClientHost, event: &mut ItemTooltipEvent<'_>) {
        let rifle = RIFLE_KEY.get().copied();
        let cartridge = CARTRIDGE_KEY.get().copied();
        let item = event.stack.item();
        let is_rifle = rifle == Some(item);
        let is_cartridge = cartridge == Some(item);
        if !is_rifle && !is_cartridge {
            return;
        }

        // Always hide Vanilla's internal `"item.durability"` line on the Winchester (`F3+H`),
        // because stack `damage` encodes loaded `.44-40` rounds (`0..=8`), not wear & tear.
        if is_rifle {
            event.lines.retain(|(line, _)| !text_contains_durability(line));
        }

        if !client_settings().hud_rich_tooltips {
            return;
        }

        let insert_at = if event.lines.is_empty() { 0 } else { 1 };
        let mut extra: Vec<(Text, u32)> = Vec::with_capacity(4);

        if is_rifle {
            let loaded = rounds(event.stack);
            let reserve = LOCAL_RESERVE_AMMO.load(Ordering::Relaxed);
            let loaded_color = if loaded == 0 { "#FF6E59" } else { "#F2D56F" };
            let loaded_arg = styled_text(Text::literal(loaded.to_string()), loaded_color, false);
            let cap_arg = styled_text(Text::literal(TUBE_CAPACITY.to_string()), "#E6D6B8", false);
            let reserve_arg = if reserve == u32::MAX {
                styled_text(Text::literal("∞"), "#FFE082", false)
            } else if reserve == 0 {
                styled_text(Text::literal("0"), "#D96B59", false)
            } else {
                styled_text(Text::literal(reserve.to_string()), "#E6D6B8", false)
            };

            extra.push((
                styled_text(
                    Text::translate("tooltip.just_a_rifle.winchester.subtitle", Vec::new()),
                    "#B8A88A",
                    true,
                ),
                u32::MAX,
            ));
            extra.push((
                styled_text(
                    Text::translate(
                        "tooltip.just_a_rifle.winchester.ammo",
                        vec![loaded_arg, cap_arg, reserve_arg],
                    ),
                    "#C8BEA8",
                    false,
                ),
                u32::MAX,
            ));
            extra.push((
                styled_text(
                    Text::translate("tooltip.just_a_rifle.winchester.stats", Vec::new()),
                    "#969CA6",
                    false,
                ),
                u32::MAX,
            ));
            extra.push((
                styled_text(
                    Text::translate(
                        "tooltip.just_a_rifle.winchester.controls",
                        vec![
                            vanilla_key_component("key.use"),
                            mod_key_component(host, RELOAD_MAPPING, "R"),
                            mod_key_component(host, INSPECT_MAPPING, "I"),
                            vanilla_key_component("key.swapOffhand"),
                        ],
                    ),
                    "#857E72",
                    false,
                ),
                u32::MAX,
            ));
        } else if is_cartridge {
            extra.push((
                styled_text(
                    Text::translate("tooltip.just_a_rifle.cartridge.caliber", Vec::new()),
                    "#D9B038",
                    true,
                ),
                u32::MAX,
            ));
            extra.push((
                styled_text(
                    Text::translate(
                        "tooltip.just_a_rifle.cartridge.usage",
                        vec![mod_key_component(host, RELOAD_MAPPING, "R")],
                    ),
                    "#A89F8D",
                    false,
                ),
                u32::MAX,
            ));
        }

        event.lines.splice(insert_at..insert_at, extra);
    }

    fn container_loot_filled(
        &self,
        level: &dyn LevelHost,
        event: &mut ContainerLootFilledEvent<'_>,
    ) {
        on_container_loot_filled(level, event);
    }

    fn living_death(&self, host: &mut dyn ServerHost, event: &mut LivingDeathEvent<'_>) -> Flow {
        let now = SERVER_TICKS.load(Ordering::Relaxed);
        handle_living_death_advancements(host, event, now);
        Flow::Continue
    }

    fn setting_changed(&self, host: &mut dyn ClientHost, _: &mut SettingChangedEvent<'_>) {
        refresh_client_settings(host);
        let cfg = client_settings();
        if cfg.ads_mode == AdsMode::Hold
            && TOGGLE_AIM_LATCH.swap(false, Ordering::Relaxed)
        {
            AIMING.store(false, Ordering::Relaxed);
            let _ = host.set_mapping_down("key.use", false);
        }
        if !cfg.blast_concussion && CONCUSSION_ACTIVE.swap(false, Ordering::Relaxed) {
            *PENDING_CONCUSSION.lock().unwrap() = None;
            let _ = host.muffle_sounds(0.0, 0.0, 0, &[]);
        }
    }

    fn play_level_sound_at_position(
        &self,
        host: HostSide<'_>,
        event: &mut PlayLevelSoundAtPositionEvent,
    ) -> Flow {
        if !matches!(host, HostSide::Client(_)) {
            return Flow::Continue;
        }
        let cfg = client_settings();
        match classify_incoming_level_sound(
            &cfg,
            event.sound.namespace(),
            event.sound.path(),
            *event.source,
            event.pitch,
        ) {
            LevelSoundDecision::Ignore => Flow::Continue,
            LevelSoundDecision::Cancel => Flow::Cancel,
            LevelSoundDecision::Mix(channel) => {
                match mix_client_sound(&cfg, channel, event.volume, event.pitch) {
                    Some((vol, pitch)) => {
                        event.volume = vol;
                        event.pitch = pitch;
                        Flow::Continue
                    }
                    None => Flow::Cancel,
                }
            }
        }
    }

    fn model_baking_completed(&self, host: &mut dyn ClientHost, _: &mut ModelBakingCompletedEvent) {
        refresh_rifle_models(host);
    }

    fn client_setup(&self, host: &mut dyn ClientHost, _: &mut ClientSetupEvent) {
        let _ = PLAYER_ARMS.set(PlayerArms::bake(host));
        refresh_rifle_models(host);
    }

    fn client_resources_reloaded(
        &self,
        host: &mut dyn ClientHost,
        _: &mut ClientResourcesReloadedEvent,
    ) {
        invalidate_skin_sprite_cache();
        refresh_rifle_models(host);
    }

    /// Left click while holding or aiming the rifle fires a round (`Flow::Cancel` suppresses
    /// vanilla mining and melee punches); optional Toggle-ADS mode latches iron sights on right click.
    fn key_mapping_pre(
        &self,
        host: &mut dyn ClientHost,
        event: &mut KeyMappingPreEvent<'_>,
    ) -> Flow {
        let holding = holding_rifle_recently() || AIMING.load(Ordering::Relaxed);
        if !holding || !host.playing() {
            return Flow::Continue;
        }
        if event.mapping == ATTACK_MAPPING {
            if event.pressed {
                let mut cancelled_inspect = false;
                let allow_fire = {
                    let mut anim = client_anim().lock().unwrap();
                    match anim.action {
                        ActionAnim::Draw { .. }
                        | ActionAnim::FireCycle { .. }
                        | ActionAnim::ReloadEndNorm { .. }
                        | ActionAnim::ReloadEndEmpty { .. } => false,
                        ActionAnim::Inspect { .. } => {
                            anim.action = ActionAnim::None;
                            cancelled_inspect = true;
                            true
                        }
                        _ => true,
                    }
                };
                if cancelled_inspect {
                    cancel_action_sound(host, Some(ActionSoundTag::Inspect), 2);
                }
                if allow_fire {
                    PENDING_FIRE.store(true, Ordering::Relaxed);
                }
            }
            return Flow::Cancel;
        }
        if event.mapping == "key.use" && client_settings().ads_mode == AdsMode::Toggle {
            if event.pressed {
                if TOGGLE_AIM_LATCH.load(Ordering::Relaxed) || AIMING.load(Ordering::Relaxed) {
                    TOGGLE_AIM_LATCH.store(false, Ordering::Relaxed);
                    AIMING.store(false, Ordering::Relaxed);
                    let _ = host.set_mapping_down("key.use", false);
                    return Flow::Cancel;
                } else if !CLIENT_RELOADING.load(Ordering::Relaxed)
                    && !CLIENT_SPRINTING.load(Ordering::Relaxed)
                {
                    TOGGLE_AIM_LATCH.store(true, Ordering::Relaxed);
                }
            } else if TOGGLE_AIM_LATCH.load(Ordering::Relaxed) {
                return Flow::Cancel;
            }
        }
        Flow::Continue
    }

    fn client_tick_pre(&self, host: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
        let tick = CLIENT_TICKS.fetch_add(1, Ordering::Relaxed) + 1;
        refresh_settings_if_dirty(host);
        let cfg = client_settings();
        if AIMING.load(Ordering::Relaxed)
            && tick.saturating_sub(AIM_LAST_TICK.load(Ordering::Relaxed)) > 2
        {
            AIMING.store(false, Ordering::Relaxed);
        }

        let Some(input) = RIFLE_INPUT.get() else {
            return;
        };

        if !host.playing() {
            clear_all_tracked_sounds(host);
            *PENDING_CONCUSSION.lock().unwrap() = None;
            if CONCUSSION_ACTIVE.swap(false, Ordering::Relaxed) {
                let _ = host.muffle_sounds(0.0, 0.0, 0, &[]);
            }
            PENDING_FIRE.store(false, Ordering::Relaxed);
            PENDING_AUTO_RELOAD.store(false, Ordering::Relaxed);
            TOGGLE_AIM_LATCH.store(false, Ordering::Relaxed);
            HEADSHOT_HITMARKER.store(false, Ordering::Relaxed);
            mark_settings_dirty();
            CLIENT_SPRINTING.store(false, Ordering::Relaxed);
            CLIENT_SNEAKING.store(false, Ordering::Relaxed);
            CLIENT_RELOADING.store(false, Ordering::Relaxed);
            LOCAL_ENTITY_ID.store(0, Ordering::Relaxed);
            LOCAL_RESERVE_AMMO.store(0, Ordering::Relaxed);
            LOCAL_OFFHAND_PACKED.store(0, Ordering::Relaxed);
            SLUNG_RIFLES.lock().unwrap().clear();
            REMOTE_TP_STATES.lock().unwrap().clear();
            crate::ballistics::clear_all_pending_impacts();
            clear_barrel_heat();
            let mut anim = client_anim().lock().unwrap();
            anim.was_holding = false;
            anim.was_slung = false;
            anim.action = ActionAnim::None;
            anim.skin_requested = false;
            return;
        }

        if !cfg.blast_concussion || cfg.vol_master <= 0.01 || cfg.vol_fire <= 0.01 {
            *PENDING_CONCUSSION.lock().unwrap() = None;
            if CONCUSSION_ACTIVE.swap(false, Ordering::Relaxed) {
                let _ = host.muffle_sounds(0.0, 0.0, 0, &[]);
            }
        } else if let Some((amount, lowpass, muffle_ticks)) =
            PENDING_CONCUSSION.lock().unwrap().take()
            && host.muffle_sounds(amount, lowpass, muffle_ticks, CONCUSSION_EXEMPT_SOUNDS)
        {
            CONCUSSION_ACTIVE.store(true, Ordering::Relaxed);
        }

        let holding = holding_rifle_recently() || AIMING.load(Ordering::Relaxed);
        let local_eid = host
            .local_player()
            .map(|p| p.entity().id())
            .or_else(|| host.camera_entity().map(|e| e.id()))
            .unwrap_or(0);
        LOCAL_ENTITY_ID.store(local_eid, Ordering::Relaxed);
        LOCAL_RESERVE_AMMO.store(count_local_reserve_ammo(host), Ordering::Relaxed);
        if tick % 20 == 0 {
            REMOTE_TP_STATES
                .lock()
                .unwrap()
                .retain(|_, v| tick.saturating_sub(v.set_tick) <= 120);
        }
        let offhand_status = local_offhand_rifle_status(host);
        let is_slung = offhand_status.is_some();
        LOCAL_OFFHAND_PACKED.store(
            match offhand_status {
                Some((off_rounds, chest_armored)) => {
                    1 | ((chest_armored as u32) << 1) | (off_rounds.min(0x3F_FFFF) << 2)
                }
                None => 0,
            },
            Ordering::Relaxed,
        );

        // Read horizontal velocity from camera entity (`Side: both`).
        let mut horiz_speed = 0.0_f32;
        let mut grounded_motion = true;
        if let Some(cam) = host.camera_entity()
            && let Some(vel) = cam.velocity(host)
        {
            horiz_speed = ((vel[0] * vel[0] + vel[2] * vel[2]).sqrt() as f32).min(0.45);
            grounded_motion = vel[1].abs() < 0.14;
        }

        let mut want_skin_request = false;
        let mut draw_foley = false;
        let mut sling_foley = false;
        let mut cancel_held_foley = false;

        {
            let mut anim = client_anim().lock().unwrap();
            if holding && !anim.skin_requested {
                anim.skin_requested = true;
                want_skin_request = true;
            }

            if holding && !anim.was_holding {
                anim.was_holding = true;
                anim.action = ActionAnim::Draw { elapsed: 0.0 };
                draw_foley = true;
            } else if !holding && anim.was_holding {
                anim.was_holding = false;
                anim.action = ActionAnim::None;
                cancel_held_foley = true;
            }

            if is_slung && !anim.was_slung {
                anim.was_slung = true;
                if !draw_foley {
                    // Dedicated quick oiled-leather & walnut back-sling foley when pressing `F`.
                    sling_foley = true;
                }
            } else if !is_slung && anim.was_slung {
                anim.was_slung = false;
            }

            // Smooth 20 Hz walk bobbing phase.
            let target_walk = if holding && grounded_motion && horiz_speed > 0.015 {
                (horiz_speed / 0.22).clamp(0.0, 1.45)
            } else {
                0.0
            };
            anim.walk_speed += (target_walk - anim.walk_speed) * 0.35;
            let sprint_now = CLIENT_SPRINTING.load(Ordering::Relaxed);
            anim.walk_phase =
                (anim.walk_phase + anim.walk_speed * if sprint_now { 0.72 } else { 0.52 }) % TAU;
        }

        if cancel_held_foley {
            cancel_action_sound(host, None, 2);
        }
        let j_tick = client_pitch_jitter(tick);
        if draw_foley {
            play_tracked_action_sound(
                host,
                ActionSoundTag::Draw,
                SoundChannel::Handling,
                SND_DRAW,
                0.85,
                1.0 + 0.02 * j_tick,
            );
        } else if sling_foley {
            let _ = play_local_foley(host, SND_SLING, 0.78, 1.0 + 0.03 * j_tick);
        }
        if want_skin_request {
            let _ = input.send(host, &RifleInput::simple(ACTION_SKIN_REQUEST));
        }

        // Always drain custom key clicks even when not holding so clicks never queue up stale.
        let mut want_reload = false;
        while host.consume_click(MOD_ID, RELOAD_MAPPING) {
            want_reload = true;
        }
        if PENDING_AUTO_RELOAD.swap(false, Ordering::Relaxed)
            && cfg.auto_reload
            && tick.saturating_sub(LAST_AUTO_RELOAD_TICK.load(Ordering::Relaxed)) >= 20
        {
            LAST_AUTO_RELOAD_TICK.store(tick, Ordering::Relaxed);
            want_reload = true;
        }
        let mut want_inspect = false;
        while host.consume_click(MOD_ID, INSPECT_MAPPING) {
            want_inspect = true;
        }

        if !holding {
            PENDING_FIRE.store(false, Ordering::Relaxed);
            if TOGGLE_AIM_LATCH.swap(false, Ordering::Relaxed) {
                let _ = host.set_mapping_down("key.use", false);
            }
            CLIENT_SPRINTING.store(false, Ordering::Relaxed);
            CLIENT_SNEAKING.store(false, Ordering::Relaxed);
            if CLIENT_RELOADING.swap(false, Ordering::Relaxed) {
                let _ = input.send(host, &RifleInput::simple(ACTION_RELOAD_CANCEL));
            }
            return;
        }

        let sneak_down = matches!(host.mapping("key.sneak"), Ok(Some(m)) if m.down);
        let sprint_down = matches!(host.mapping("key.sprint"), Ok(Some(m)) if m.down);
        let forward_down = matches!(host.mapping("key.forward"), Ok(Some(m)) if m.down);
        let sprinting = sprint_down && forward_down && !sneak_down;
        CLIENT_SNEAKING.store(sneak_down, Ordering::Relaxed);
        CLIENT_SPRINTING.store(sprinting, Ordering::Relaxed);

        if sprinting {
            TOGGLE_AIM_LATCH.store(false, Ordering::Relaxed);
            if AIMING.swap(false, Ordering::Relaxed) {
                let _ = host.set_mapping_down("key.use", false);
            }
            if CLIENT_RELOADING.swap(false, Ordering::Relaxed) {
                PENDING_RELOAD_CANCEL.store(true, Ordering::Relaxed);
            }
            let cancelled = {
                let mut anim = client_anim().lock().unwrap();
                if matches!(
                    anim.action,
                    ActionAnim::Inspect { .. }
                        | ActionAnim::ReloadIntro { .. }
                        | ActionAnim::ReloadInsert { .. }
                ) {
                    anim.action = ActionAnim::None;
                    true
                } else {
                    false
                }
            };
            if cancelled {
                cancel_action_sound(host, None, 2);
            }
        }

        if cfg.ads_mode == AdsMode::Toggle && TOGGLE_AIM_LATCH.load(Ordering::Relaxed) {
            if CLIENT_RELOADING.load(Ordering::Relaxed) || sprinting {
                TOGGLE_AIM_LATCH.store(false, Ordering::Relaxed);
                let _ = host.set_mapping_down("key.use", false);
            } else {
                let _ = host.set_mapping_down("key.use", true);
            }
        }

        if AIMING.load(Ordering::Relaxed) {
            let cancelled_inspect = {
                let mut anim = client_anim().lock().unwrap();
                if matches!(anim.action, ActionAnim::Inspect { .. }) {
                    anim.action = ActionAnim::None;
                    true
                } else {
                    false
                }
            };
            if cancelled_inspect {
                cancel_action_sound(host, Some(ActionSoundTag::Inspect), 3);
            }
        }

        if CLIENT_RELOADING.load(Ordering::Relaxed)
            && (AIMING.swap(false, Ordering::Relaxed)
                || TOGGLE_AIM_LATCH.swap(false, Ordering::Relaxed))
        {
            let _ = host.set_mapping_down("key.use", false);
        }

        if PENDING_RELOAD_CANCEL.swap(false, Ordering::Relaxed) {
            let _ = input.send(host, &RifleInput::simple(ACTION_RELOAD_CANCEL));
        }

        if PENDING_FIRE.swap(false, Ordering::Relaxed) {
            let _ = input.send(host, &build_fire_input());
        }

        if want_reload && !CLIENT_RELOADING.load(Ordering::Relaxed) && !sprinting {
            let busy_draw = matches!(
                client_anim().lock().unwrap().action,
                ActionAnim::Draw { .. }
            );
            if !busy_draw {
                TOGGLE_AIM_LATCH.store(false, Ordering::Relaxed);
                AIMING.store(false, Ordering::Relaxed);
                let _ = host.set_mapping_down("key.use", false);
                let _ = input.send(host, &RifleInput::simple(ACTION_RELOAD));
            }
        } else if want_inspect && !CLIENT_RELOADING.load(Ordering::Relaxed) && !sprinting {
            let start_inspect = {
                let mut anim = client_anim().lock().unwrap();
                if anim.action == ActionAnim::None && !AIMING.load(Ordering::Relaxed) {
                    anim.action = ActionAnim::Inspect { elapsed: 0.0 };
                    true
                } else {
                    false
                }
            };
            if start_inspect {
                play_tracked_action_sound(
                    host,
                    ActionSoundTag::Inspect,
                    SoundChannel::Handling,
                    SND_INSPECT,
                    0.90,
                    1.0,
                );
            }
        }
    }

    fn server_tick(&self, _: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
        if matches!(event.phase, ServerTickPhase::ServerTickEnd { .. }) {
            SERVER_TICKS.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn level_tick_post(&self, host: HostSide<'_>, _: &mut LevelTickPostEvent) {
        let HostSide::Server(host) = host else {
            return;
        };
        let now = SERVER_TICKS.load(Ordering::Relaxed);
        tick_pending_impacts();
        tick_reload_sessions(host, now);
        tick_active_bullets(host, now);
        tick_barrel_heat(host, now);
        flush_pending_exploration_advancements(host);
    }

    /// Per-frame animation stepper (`event.elapsed`) + 240Hz camera recoil climb & partial recovery.
    fn render_frame_pre(&self, host: &mut dyn ClientHost, event: &mut RenderFramePreEvent) {
        FRAME_SEQ.fetch_add(1, Ordering::Relaxed);
        if host.is_paused() {
            return;
        }
        let dt = event.elapsed.as_secs_f32().clamp(0.0, 0.05);
        if dt <= 0.0 {
            return;
        }
        let cfg = client_settings();

        let hit_rem = f32::from_bits(HITMARKER_TIMER_BITS.load(Ordering::Relaxed));
        if hit_rem > 0.0 {
            HITMARKER_TIMER_BITS.store((hit_rem - dt).max(0.0).to_bits(), Ordering::Relaxed);
        }

        // Step world decals & tumbling/bouncing .44-40 brass casings even when the player switches
        // away from the rifle so airborne shells finish landing and clinking naturally.
        step_world_effects(host, dt);

        let holding = holding_rifle_recently() || AIMING.load(Ordering::Relaxed);
        let sprinting = CLIENT_SPRINTING.load(Ordering::Relaxed);

        let mut cam_delta_pitch = 0.0_f32;
        let mut cam_delta_yaw = 0.0_f32;
        let mut triggered_foley: Vec<(SoundChannel, &'static str, f32, f32)> = Vec::new();
        let mut eject_casing_aim: Option<f32> = None;

        {
            let mut anim = client_anim().lock().unwrap();
            if !holding {
                anim.aim_dyn.reset(0.0);
                anim.recoil_dyn.reset(0.0);
                anim.sprint_dyn.reset(0.0);
                anim.recoil_target_timer = None;
                anim.cam_recoil_timer = None;
                anim.jolt_timer = 0.0;
                ADS_BITS.store(0.0_f32.to_bits(), Ordering::Relaxed);
                return;
            }

            let aiming_want = AIMING.load(Ordering::Relaxed)
                && !CLIENT_RELOADING.load(Ordering::Relaxed)
                && !sprinting
                && !anim.blocks_ads();

            let aim_y = anim.aim_dyn.step(dt, if aiming_want { 1.0 } else { 0.0 });
            ADS_BITS.store(aim_y.clamp(0.0, 1.2).to_bits(), Ordering::Relaxed);

            let sprint_want = sprinting && !aiming_want && anim.action == ActionAnim::None;
            let _ = anim.sprint_dyn.step(dt, if sprint_want { 1.0 } else { 0.0 });

            anim.idle_time = (anim.idle_time + dt) % 600.0;

            let kick_target = if let Some(cur) = anim.recoil_target_timer {
                let next = cur + dt;
                anim.recoil_target_timer = if next >= 0.22 { None } else { Some(next) };
                recoil_envelope(cur)
            } else {
                0.0
            };
            let _ = anim.recoil_dyn.step(dt, kick_target);

            if let Some(t0) = anim.cam_recoil_timer {
                let t1 = t0 + dt;
                let (cp0, cy0) = cam_recoil_curve(t0);
                let (cp1, cy1) = cam_recoil_curve(t1);
                cam_delta_pitch = (cp1 - cp0) * anim.cam_pitch_kick;
                cam_delta_yaw = (cy1 - cy0) * anim.cam_yaw_kick;
                anim.cam_recoil_timer = if t1 >= 0.385 { None } else { Some(t1) };
            }

            if anim.jolt_timer > 0.0 {
                anim.jolt_timer = (anim.jolt_timer - dt).max(0.0);
            }

            // Advance TAC-style action animation state & trigger frame-exact stereo foley.
            match anim.action {
                ActionAnim::None => {}
                ActionAnim::Draw { elapsed } => {
                    let next = elapsed + dt;
                    anim.action = if next >= DRAW_DURATION_SECS {
                        ActionAnim::None
                    } else {
                        ActionAnim::Draw { elapsed: next }
                    };
                }
                ActionAnim::FireCycle {
                    elapsed,
                    keep_uncocked,
                    mut lever_sound_played,
                    mut cock_sound_played,
                    mut casing_ejected,
                } => {
                    let next = elapsed + dt;
                    if keep_uncocked {
                        anim.hammer_uncocked = true;
                        if !casing_ejected && next >= CASING_LAST_HANDOFF_SECS {
                            casing_ejected = true;
                            eject_casing_aim = Some(aim_y.clamp(0.0, 1.0));
                        }
                        anim.action = if next >= 0.26 {
                            ActionAnim::None
                        } else {
                            ActionAnim::FireCycle {
                                elapsed: next,
                                keep_uncocked: true,
                                lever_sound_played: true,
                                cock_sound_played: true,
                                casing_ejected,
                            }
                        };
                    } else {
                        let j = client_pitch_jitter(FRAME_SEQ.load(Ordering::Relaxed));
                        if !lever_sound_played && next >= LEVER_DELAY_SECS {
                            lever_sound_played = true;
                            triggered_foley.push((
                                SoundChannel::Mechanism,
                                SND_LEVER,
                                0.86,
                                1.0 + 0.025 * j,
                            ));
                        }
                        if !casing_ejected && next >= CASING_HANDOFF_SECS {
                            casing_ejected = true;
                            eject_casing_aim = Some(aim_y.clamp(0.0, 1.0));
                        }
                        if !cock_sound_played && next >= LEVER_DELAY_SECS + 0.24 {
                            cock_sound_played = true;
                            anim.hammer_uncocked = false;
                            triggered_foley.push((
                                SoundChannel::Mechanism,
                                SND_COCK,
                                0.72,
                                1.04 + 0.025 * j,
                            ));
                        }
                        let total = LEVER_DELAY_SECS + LEVER_DURATION_SECS;
                        anim.action = if next >= total {
                            anim.hammer_uncocked = false;
                            ActionAnim::None
                        } else {
                            ActionAnim::FireCycle {
                                elapsed: next,
                                keep_uncocked: false,
                                lever_sound_played,
                                cock_sound_played,
                                casing_ejected,
                            }
                        };
                    }
                }
                ActionAnim::DryClick { elapsed } => {
                    let next = elapsed + dt;
                    anim.action = if next >= 0.22 {
                        ActionAnim::None
                    } else {
                        ActionAnim::DryClick { elapsed: next }
                    };
                }
                ActionAnim::ReloadIntro { elapsed } => {
                    let next = elapsed + dt;
                    anim.action = if next > 2.5 {
                        ActionAnim::None
                    } else {
                        ActionAnim::ReloadIntro { elapsed: next }
                    };
                }
                ActionAnim::ReloadInsert { elapsed } => {
                    let next = elapsed + dt;
                    anim.action = if next > 2.5 {
                        ActionAnim::None
                    } else {
                        ActionAnim::ReloadInsert { elapsed: next }
                    };
                }
                ActionAnim::ReloadEndNorm { elapsed } => {
                    let next = elapsed + dt;
                    let dur = (RELOAD_CLOSE_NORM_TICKS as f32) * 0.05;
                    anim.action = if next >= dur {
                        ActionAnim::None
                    } else {
                        ActionAnim::ReloadEndNorm { elapsed: next }
                    };
                }
                ActionAnim::ReloadEndEmpty {
                    elapsed,
                    mut lever_sound_played,
                    mut cock_sound_played,
                } => {
                    let next = elapsed + dt;
                    let dur = (RELOAD_CLOSE_EMPTY_TICKS as f32) * 0.05;
                    let j = client_pitch_jitter(FRAME_SEQ.load(Ordering::Relaxed) ^ 0x55AA);
                    if !lever_sound_played && next >= 0.15 {
                        lever_sound_played = true;
                        triggered_foley.push((
                            SoundChannel::Mechanism,
                            SND_LEVER,
                            0.90,
                            0.98 + 0.02 * j,
                        ));
                    }
                    if !cock_sound_played && next >= 0.40 {
                        cock_sound_played = true;
                        anim.hammer_uncocked = false;
                        triggered_foley.push((
                            SoundChannel::Mechanism,
                            SND_COCK,
                            0.78,
                            1.0 + 0.025 * j,
                        ));
                    }
                    anim.action = if next >= dur {
                        anim.hammer_uncocked = false;
                        ActionAnim::None
                    } else {
                        ActionAnim::ReloadEndEmpty {
                            elapsed: next,
                            lever_sound_played,
                            cock_sound_played,
                        }
                    };
                }
                ActionAnim::Inspect { elapsed } => {
                    let next = elapsed + dt;
                    anim.action = if next >= INSPECT_DURATION_SECS {
                        ActionAnim::None
                    } else {
                        ActionAnim::Inspect { elapsed: next }
                    };
                }
            }
        }

        if let Some(aim) = eject_casing_aim {
            spawn_spent_casing_from_camera(host, aim);
        }

        for (ch, snd, vol, pitch) in triggered_foley {
            let _ = play_local_foley_channel(host, ch, snd, vol, pitch);
        }

        cam_delta_pitch *= cfg.camera_recoil_mul;
        cam_delta_yaw *= cfg.camera_recoil_mul;
        if (cam_delta_pitch.abs() > 1e-5 || cam_delta_yaw.abs() > 1e-5)
            && let Some(cam) = host.camera_entity()
            && let Some([yaw, pitch]) = cam.rotation(host)
        {
            let new_pitch = (pitch - cam_delta_pitch).clamp(-89.9, 89.9);
            let new_yaw = yaw + cam_delta_yaw;
            let _ = host.set_local_rotation(new_yaw, new_pitch);
        }
    }

    fn submit_custom_geometry(&self, _: &dyn ClientHost, event: &mut SubmitCustomGeometryEvent<'_>) {
        submit_world_effects(event.draw);
    }

    fn compute_fov(&self, _: &dyn ClientHost, event: &mut ComputeFovEvent) {
        let aim = aim_progress().clamp(0.0, 1.15);
        if aim > 0.001 {
            let zoom_strength = (1.0 - AIM_FOV) * client_settings().ads_zoom_mul;
            let scale = (1.0 - zoom_strength * aim).clamp(0.35, 1.0);
            let scaled = (event.degrees * scale).clamp(1.0, 179.0);
            if scaled.is_finite() {
                event.degrees = scaled;
            }
        }
        if event.degrees.is_finite() {
            LAST_WORLD_FOV_BITS.store(event.degrees.to_bits(), Ordering::Relaxed);
        }
    }

    fn calculate_player_turn(&self, _: &dyn ClientHost, event: &mut CalculatePlayerTurnEvent) {
        let aim = aim_progress().clamp(0.0, 1.0);
        if aim > 0.001 {
            let base_ads_sens = (1.0 - (1.0 - AIM_FOV) * 0.85) * client_settings().ads_sensitivity_mul;
            let sens_scale = (1.0 + (base_ads_sens - 1.0) * aim).clamp(0.05, 2.0);
            event.mouse_sensitivity *= f64::from(sens_scale);
        }
    }

    fn camera_pose(&self, _: &dyn ClientHost, event: &mut CameraPoseEvent<'_>) {
        let recoil_mul = client_settings().camera_recoil_mul;
        if recoil_mul <= 0.001 {
            return;
        }
        let (jolt, sign) = {
            let anim = client_anim().lock().unwrap();
            (anim.jolt_timer, anim.jolt_yaw_sign)
        };
        if jolt > 0.0 {
            let u = (jolt / 0.065).clamp(0.0, 1.0);
            let wave = (PI * (1.0 - u) * 2.0).sin() * u * recoil_mul;
            event.pitch -= 0.42 * wave;
            event.yaw += 0.24 * wave * sign;
        }
    }

    fn get_arm_pose(
        &self,
        host: &dyn ClientHost,
        event: &mut GetArmPoseEvent<'_>,
    ) -> Option<ArmPose> {
        let rifle = RIFLE_KEY.get().copied()?;
        let now = CLIENT_TICKS.load(Ordering::Relaxed);
        {
            let mut slung = SLUNG_RIFLES.lock().unwrap();
            if let Some(off) = event.humanoid.off_hand.filter(|s| s.item() == rifle) {
                slung.insert(
                    event.entity_id,
                    SlungRifleState {
                        last_tick: now,
                        hammer_uncocked: rounds(off) == 0,
                        main_arm_left: event.humanoid.main_arm_left,
                    },
                );
            } else {
                slung.remove(&event.entity_id);
            }
        }

        if event.hand == Hand::Off && event.stack.item() == rifle {
            // Keep the off-hand arm relaxed at the player's side while the rifle is slung on the back.
            return Some(ArmPose::Empty);
        }

        if event.hand == Hand::Main && event.stack.item() == rifle {
            let local_id = LOCAL_ENTITY_ID.load(Ordering::Relaxed);
            let is_local = local_id != 0 && event.entity_id == local_id;

            let (reloading, lever_cycling, aiming_ads, sprinting) = if is_local {
                let action = client_anim().lock().unwrap().action;
                let cyc = matches!(
                    action,
                    ActionAnim::FireCycle {
                        keep_uncocked: false,
                        elapsed,
                        ..
                    } if elapsed >= LEVER_DELAY_SECS * 0.6
                ) || matches!(
                    action,
                    ActionAnim::ReloadEndEmpty { elapsed, .. } if elapsed >= 0.15
                );
                let rel = !cyc
                    && (CLIENT_RELOADING.load(Ordering::Relaxed)
                        || matches!(
                            action,
                            ActionAnim::ReloadIntro { .. }
                                | ActionAnim::ReloadInsert { .. }
                                | ActionAnim::ReloadEndNorm { .. }
                                | ActionAnim::ReloadEndEmpty { .. }
                        ));
                let ads = (AIMING.load(Ordering::Relaxed)
                    || aim_progress() > 0.35
                    || event.humanoid.use_hand == Some(Hand::Main))
                    && !rel
                    && !cyc;
                let spr = CLIENT_SPRINTING.load(Ordering::Relaxed) && !ads && !rel;
                (rel, cyc, ads, spr)
            } else {
                let remote = REMOTE_TP_STATES
                    .lock()
                    .unwrap()
                    .get(&event.entity_id)
                    .copied();
                let rel = matches!(
                    remote,
                    Some(r) if r.state == TP_STATE_RELOADING && now.saturating_sub(r.set_tick) <= 120
                );
                let cyc = matches!(
                    remote,
                    Some(r) if r.state == TP_STATE_LEVER_CYCLE && now.saturating_sub(r.set_tick) <= 11
                );
                let ads = event.humanoid.use_hand == Some(Hand::Main) && !rel && !cyc;
                let spr = !ads
                    && !rel
                    && host
                        .entity(event.entity_id)
                        .and_then(|e| e.velocity(host))
                        .is_some_and(|[vx, _, vz]| vx * vx + vz * vz > 0.055);
                (rel, cyc, ads, spr)
            };

            if reloading {
                return POSE_RELOAD.get().copied().map(ArmPose::Custom);
            }
            if lever_cycling {
                return POSE_LEVER_CYCLE.get().copied().map(ArmPose::Custom);
            }
            if aiming_ads {
                return Some(ArmPose::CrossbowHold);
            }
            if sprinting {
                return POSE_SPRINT_CARRY.get().copied().map(ArmPose::Custom);
            }
            return POSE_HIP_READY.get().copied().map(ArmPose::Custom);
        }
        None
    }

    fn arm_pose_transform(&self, _: &dyn ClientHost, event: &mut ArmPoseTransformEvent<'_>) {
        let left_handed = event.humanoid.main_arm_left;
        let base_swing = if left_handed {
            event.angles.left[0]
        } else {
            event.angles.right[0]
        };
        let stride = (base_swing * 0.14).clamp(-0.10, 0.10);
        let t = (CLIENT_TICKS.load(Ordering::Relaxed) as f32) * 0.05;

        let (main_rh, support_rh) = if POSE_RELOAD.get().copied() == Some(event.pose) {
            let thumb = (t * 12.0).sin();
            (
                [-1.06 + 0.15 * thumb, -0.44 - 0.08 * thumb, -0.18],
                [-1.14 + stride * 0.4, 0.58, -0.16],
            )
        } else if POSE_LEVER_CYCLE.get().copied() == Some(event.pose) {
            let cycle = (t * 10.5).sin() * 0.5 + 0.5;
            (
                [-0.46 + 0.22 * cycle + stride * 0.5, -0.14, 0.12],
                [-1.08 + stride * 0.5, 0.56, -0.10],
            )
        } else if POSE_SPRINT_CARRY.get().copied() == Some(event.pose) {
            (
                [-0.92 + stride * 1.4, -0.36, 0.14],
                [-1.20 + stride * 1.4, 0.68, -0.14],
            )
        } else if POSE_HIP_READY.get().copied() == Some(event.pose) {
            (
                [-0.66 + stride, -0.16, 0.06],
                [-1.00 + stride, 0.52, -0.08],
            )
        } else {
            return;
        };

        if left_handed {
            event.angles.left = [main_rh[0], -main_rh[1], -main_rh[2]];
            event.angles.right = [support_rh[0], -support_rh[1], -support_rh[2]];
        } else {
            event.angles.right = main_rh;
            event.angles.left = support_rh;
        }
    }

    fn render_player_post(&self, _: &dyn ClientHost, event: &mut RenderPlayerPostEvent<'_>) {
        if event.player.death_time > 0 {
            return;
        }
        let Some(body_part) = event
            .parts
            .as_ref()
            .and_then(|parts| parts.get(HumanoidPart::Body))
        else {
            return;
        };
        if !body_part.visible {
            return;
        }

        let now = CLIENT_TICKS.load(Ordering::Relaxed);
        let cached = SLUNG_RIFLES
            .lock()
            .unwrap()
            .get(&event.player.id)
            .copied()
            .filter(|s| now.saturating_sub(s.last_tick) <= 4);

        let (hammer_uncocked, main_arm_left, back_z_tex) = if event.who.local {
            let packed = LOCAL_OFFHAND_PACKED.load(Ordering::Relaxed);
            if packed == 0 {
                return;
            }
            let sling_style = client_settings().back_sling_style;
            if sling_style == BackSlingStyle::Hidden {
                return;
            }
            let chest_armored = (packed & 0b10) != 0;
            let off_rounds = packed >> 2;
            let raw_left = cached.is_some_and(|s| s.main_arm_left);
            let left = if sling_style == BackSlingStyle::FlipShoulder {
                !raw_left
            } else {
                raw_left
            };
            let z_tex = if chest_armored {
                BACK_SLING_ARMOR_Z_TEX
            } else {
                BACK_SLING_JACKET_Z_TEX
            };
            (off_rounds == 0, left, z_tex)
        } else if let Some(state) = cached {
            (
                state.hammer_uncocked,
                state.main_arm_left,
                BACK_SLING_REMOTE_Z_TEX,
            )
        } else {
            return;
        };

        let Some(models) = *RIFLE_MODELS.lock().unwrap() else {
            return;
        };

        let body = body_part.pose;
        let draw = &mut *event.draw;
        draw.push();

        // 1. Match AvatarRenderer::base_transform (`Ry(180 - yaw) * S(-1, -1, 1)` == `Ry(-yaw) * Rx(180)`, det = +1).
        draw.rotate_about([1.0, 0.0, 0.0], 180.0, [0.0; 3]);
        draw.scale([AVATAR_RENDER_SCALE; 3]);
        if event.player.baby {
            draw.scale([0.5; 3]);
        }
        draw.translate([0.0, AVATAR_MODEL_OFFSET_Y, 0.0]);

        // 2. Follow HumanoidPart::Body PartPose (`ModelPart.translateAndRotate`: T(p/16) * Rz * Ry * Rx * S).
        draw.translate([body.x / 16.0, body.y / 16.0, body.z / 16.0]);
        draw.rotate_about([0.0, 0.0, 1.0], body.z_rot.to_degrees(), [0.0; 3]);
        draw.rotate_about([0.0, 1.0, 0.0], body.y_rot.to_degrees(), [0.0; 3]);
        draw.rotate_about([1.0, 0.0, 0.0], body.x_rot.to_degrees(), [0.0; 3]);
        draw.scale([body.x_scale, body.y_scale, body.z_scale]);

        // 3. Return to upright torso space (+X player left, +Y up from neck, -Z behind back; det = +1).
        draw.rotate_about([1.0, 0.0, 0.0], 180.0, [0.0; 3]);

        // 4. Diagonal 44° cross-back sling: barrel rises over the main-hand shoulder, walnut buttstock
        //    angles down toward the off-hand hip, flat receiver side rests flush against the back, and
        //    the King's loading gate faces outward.
        let diag_deg = if main_arm_left {
            -BACK_SLING_DIAG_DEG
        } else {
            BACK_SLING_DIAG_DEG
        };
        let ry_deg = if main_arm_left { -90.0 } else { 90.0 };
        draw.translate([0.0, BACK_SLING_OFFSET_Y_TEX / 16.0, -back_z_tex / 16.0]);
        draw.rotate_about([0.0, 0.0, 1.0], diag_deg, [0.0; 3]);
        draw.rotate_about([0.0, 1.0, 0.0], ry_deg, [0.0; 3]);
        draw.scale([BACK_SLING_MODEL_SCALE; 3]);
        draw.translate([
            -BACK_SLING_CENTER_TEX[0] / 16.0,
            -BACK_SLING_CENTER_TEX[1] / 16.0,
            -BACK_SLING_CENTER_TEX[2] / 16.0,
        ]);

        let hammer_deg = if hammer_uncocked { -34.0 } else { 0.0 };
        let _ = draw_back_sling_assembly(draw, models, hammer_deg);
        draw.pop();
    }

    fn render_hand(&self, _: &dyn ClientHost, event: &mut RenderHandEvent<'_, '_>) -> Flow {
        let Some(rifle) = RIFLE_KEY.get().copied() else {
            return Flow::Continue;
        };

        let hand = event.render.hand;
        if hand == Hand::Off {
            let off_is_rifle = event.render.stack.is_some_and(|s| s.item() == rifle);
            if off_is_rifle || holding_rifle_recently() {
                // Off-hand Winchester is slung diagonally across the player's back,
                // and main-hand Winchester occupies both hands.
                return Flow::Cancel;
            }
            return Flow::Continue;
        }

        let Some(loaded_on_stack) = event
            .render
            .stack
            .filter(|s| s.item() == rifle)
            .map(rounds)
        else {
            RIFLE_SEEN_FRAME.store(0, Ordering::Relaxed);
            return Flow::Continue;
        };

        let equip_raw = event.render.equip;
        RIFLE_SEEN_FRAME.store(FRAME_SEQ.load(Ordering::Relaxed), Ordering::Relaxed);

        let (
            aim,
            kick,
            sprint_blend,
            ads_factor,
            crouch_factor,
            roll_sign,
            walk_phase,
            walk_speed,
            idle_time,
            hammer_uncocked,
            action,
            skin,
        ) = {
            let mut anim = client_anim().lock().unwrap();
            anim.known_rounds = loaded_on_stack;
            if loaded_on_stack == 0 && !matches!(anim.action, ActionAnim::ReloadEndEmpty { .. }) {
                anim.hammer_uncocked = true;
            }
            (
                anim.aim_dyn.y,
                anim.recoil_dyn.y,
                anim.sprint_dyn.y.clamp(0.0, 1.0),
                anim.shot_ads_factor,
                anim.shot_crouch_factor,
                anim.shot_roll_sign,
                anim.walk_phase,
                anim.walk_speed,
                anim.idle_time,
                anim.hammer_uncocked,
                anim.action,
                anim.skin.clone(),
            )
        };

        let equip = equip_raw.clamp(0.0, 1.0) * (1.0 - aim.clamp(0.0, 1.0));
        let sway_mul = client_settings().weapon_sway_mul;
        let pose = evaluate_weapon_pose(
            aim,
            equip,
            kick,
            sprint_blend,
            ads_factor,
            crouch_factor,
            roll_sign,
            walk_phase,
            walk_speed,
            idle_time,
            sway_mul,
            hammer_uncocked,
            action,
        );
        let muzzle_view = compute_gun_texel_view_pos(MUZZLE_TEXELS, &pose, equip);
        LAST_MUZZLE_VIEW_X.store(muzzle_view[0].to_bits(), Ordering::Relaxed);
        LAST_MUZZLE_VIEW_Y.store(muzzle_view[1].to_bits(), Ordering::Relaxed);
        LAST_MUZZLE_VIEW_Z.store(muzzle_view[2].to_bits(), Ordering::Relaxed);

        let ejector_view = compute_gun_texel_view_pos(EJECTOR_HANDOFF_TEXELS, &pose, equip);
        LAST_EJECTOR_VIEW_X.store(ejector_view[0].to_bits(), Ordering::Relaxed);
        LAST_EJECTOR_VIEW_Y.store(ejector_view[1].to_bits(), Ordering::Relaxed);
        LAST_EJECTOR_VIEW_Z.store(ejector_view[2].to_bits(), Ordering::Relaxed);

        let arms = PLAYER_ARMS.get_or_init(PlayerArms::procedural_both);
        let arm_pair = if skin.slim { &arms.slim } else { &arms.wide };
        let models_opt = *RIFLE_MODELS.lock().unwrap();

        if let Some(models) = models_opt {
            let draw = &mut *event.draw;
            draw.push();

            // 1. Animated weapon root pose in camera view space.
            draw.translate(pose.gun_trans);
            draw.rotate_about([1.0, 0.0, 0.0], pose.gun_rot[0], [0.0; 3]);
            draw.rotate_about([0.0, 1.0, 0.0], pose.gun_rot[1], [0.0; 3]);
            draw.rotate_about([0.0, 0.0, 1.0], pose.gun_rot[2], [0.0; 3]);

            // 2. Vanilla right-hand anchor (`HAND_BASE`) + `display.firstperson_righthand`
            //    (`RIFLE_DISPLAY_TRANS`, `0.60` scale) + `[-0.5; 3]` unit-block centering.
            draw.translate([HAND_BASE[0], HAND_BASE[1] - equip * 0.6, HAND_BASE[2]]);
            draw.translate(RIFLE_DISPLAY_TRANS);
            draw.scale([RIFLE_DISPLAY_SCALE; 3]);
            draw.translate([-0.5, -0.5, -0.5]);

            // 3–7. Articulated 5-part Winchester assembly (`body`, `lever`, `hammer`, `trigger`, `gate`).
            if draw_rifle_assembly(
                draw,
                models,
                pose.lever_deg,
                pose.hammer_deg,
                pose.trigger_deg,
                pose.gate_deg,
            ) {
                // 8a. Brass `.44-40` cartridge pushed through the loading gate during reload.
                if pose.show_cartridge {
                    draw.push();
                    draw.translate(tex_to_unit(pose.cartridge_tex));
                    draw.rotate_about([0.0, 1.0, 0.0], pose.cartridge_rot[1], [0.0; 3]);
                    draw.rotate_about([1.0, 0.0, 0.0], pose.cartridge_rot[0], [0.0; 3]);
                    draw.rotate_about([0.0, 0.0, 1.0], pose.cartridge_rot[2], [0.0; 3]);
                    draw.translate([-0.5, -0.5, -0.5]);
                    let _ = draw.model(models.cartridge, 0xFF_FF_FF_FF);
                    draw.pop();
                }

                // 8b. Spent `.44-40` brass casing rising out of the top carrier / ejection port
                //     (`trava_dorsal`, depth-clipped by the steel receiver walls) right before
                //     0-pixel handoff to world-space physics.
                if pose.show_casing {
                    draw.push();
                    draw.translate(tex_to_unit(pose.casing_tex));
                    draw.rotate_about([0.0, 1.0, 0.0], pose.casing_rot[1], [0.0; 3]);
                    draw.rotate_about([1.0, 0.0, 0.0], pose.casing_rot[0], [0.0; 3]);
                    draw.rotate_about([0.0, 0.0, 1.0], pose.casing_rot[2], [0.0; 3]);
                    draw.translate([-0.5, -0.5, -0.5]);
                    let _ = draw.model(models.casing, 0xFF_FF_FF_FF);
                    draw.pop();
                }

                // 9. Both 1st-person player arms (`slim` 3px or `wide` 4px + outer sleeves)
                //    drawn in the same `[0..1]` rifle frame with matching proportions.
                draw_gun_space_arm(
                    draw,
                    &arm_pair.right,
                    &skin,
                    true,
                    pose.right_hand_tex,
                    pose.right_hand_rot,
                );
                draw_gun_space_arm(
                    draw,
                    &arm_pair.left,
                    &skin,
                    false,
                    pose.left_hand_tex,
                    pose.left_hand_rot,
                );

                draw.pop();
                return Flow::Cancel;
            }
            draw.pop();
        }

        // Fallback if standalone models haven't finished baking yet: pose vanilla item + arms.
        {
            let render = &mut *event.render;
            render.translation[0] += pose.gun_trans[0];
            render.translation[1] += pose.gun_trans[1];
            render.translation[2] += pose.gun_trans[2];
            render.rotation[0] += pose.gun_rot[0];
            render.rotation[1] += pose.gun_rot[1];
            render.rotation[2] += pose.gun_rot[2];
        }
        let draw = &mut *event.draw;
        draw.push();
        draw.translate(pose.gun_trans);
        draw.rotate_about([1.0, 0.0, 0.0], pose.gun_rot[0], [0.0; 3]);
        draw.rotate_about([0.0, 1.0, 0.0], pose.gun_rot[1], [0.0; 3]);
        draw.rotate_about([0.0, 0.0, 1.0], pose.gun_rot[2], [0.0; 3]);
        draw.translate([HAND_BASE[0], HAND_BASE[1] - equip * 0.6, HAND_BASE[2]]);
        draw.translate(RIFLE_DISPLAY_TRANS);
        draw.scale([RIFLE_DISPLAY_SCALE; 3]);
        draw.translate([-0.5, -0.5, -0.5]);
        draw_gun_space_arm(
            draw,
            &arm_pair.right,
            &skin,
            true,
            pose.right_hand_tex,
            pose.right_hand_rot,
        );
        draw_gun_space_arm(
            draw,
            &arm_pair.left,
            &skin,
            false,
            pose.left_hand_tex,
            pose.left_hand_rot,
        );
        draw.pop();
        Flow::Continue
    }

    /// Hide the vanilla HUD crosshair (`+` and crosshair attack indicator) while aiming down sights
    /// so the Winchester 1873 iron sights (`massa_lamina` + `alca_entalhe_base`) remain unobstructed.
    fn gui_layer_pre(&self, _: &dyn ClientHost, event: &mut GuiLayerPreEvent<'_>) -> Flow {
        if client_settings().hide_crosshair_ads
            && (event.layer == "minecraft:crosshair" || event.layer == "crosshair")
            && (AIMING.load(Ordering::Relaxed)
                || (holding_rifle_recently() && aim_progress() > 0.08))
        {
            return Flow::Cancel;
        }
        Flow::Continue
    }

    fn gui_render_post(&self, _: &dyn ClientHost, event: &mut GuiRenderPostEvent<'_>) {
        if event.gui_hidden || !holding_rifle_recently() {
            return;
        }
        let cfg = client_settings();
        let aim = aim_progress().clamp(0.0, 1.0);
        let [wf, hf] = event.batch.size();
        let (w, h) = (wf as i32, hf as i32);

        if aim >= 0.02 && cfg.ads_vignette_mul > 0.01 {
            let darken = ((1.0 - AIM_VIGNETTE) * cfg.ads_vignette_mul * aim).clamp(0.0, 0.85);
            let brightness = ((1.0 - darken) * 255.0).clamp(0.0, 255.0) as u32;
            let color = 0xFF00_0000 | (brightness << 16) | (brightness << 8) | brightness;
            let _ = event.batch.gui_paint(&[GuiDraw::Blit {
                texture: "misc/vignette".into(),
                blend: mod_api::client::hud_layout::Blend::Vignette,
                x: 0,
                y: 0,
                w,
                h,
                uv: [0.0, 0.0],
                src: [w, h],
                tex: [w, h],
                color,
            }]);
        }

        // Optional subtle brass hitmarker (or crimson-gold double tick on 1.5x headshots!).
        let hit_rem = f32::from_bits(HITMARKER_TIMER_BITS.load(Ordering::Relaxed));
        if cfg.hud_hitmarker && hit_rem > 0.001 {
            let is_headshot = HEADSHOT_HITMARKER.load(Ordering::Relaxed);
            let dur = if is_headshot { 0.22_f32 } else { 0.14_f32 };
            let alpha = ((hit_rem / dur).clamp(0.0, 1.0) * 235.0) as u32;
            let rgb = if is_headshot {
                0xFF_6A_4D
            } else {
                cfg.hud_accent_color & 0xFF_FFFF
            };
            let hm_color = (alpha << 24) | rgb;
            let cx = wf * 0.5;
            let cy = hf * 0.5;
            for step in 2..=5 {
                let d = step as f32;
                event.batch.fill([cx - d - 0.5, cy - d - 0.5, 1.2, 1.2], hm_color);
                event.batch.fill([cx + d - 0.7, cy - d - 0.5, 1.2, 1.2], hm_color);
                event.batch.fill([cx - d - 0.5, cy + d - 0.7, 1.2, 1.2], hm_color);
                event.batch.fill([cx + d - 0.7, cy + d - 0.7, 1.2, 1.2], hm_color);
            }
            if is_headshot {
                let gold_color = ((alpha * 4 / 5) << 24) | 0xFF_DF_78;
                for step in 6..=7 {
                    let d = step as f32;
                    event
                        .batch
                        .fill([cx - d - 0.5, cy - d - 0.5, 1.3, 1.3], gold_color);
                    event
                        .batch
                        .fill([cx + d - 0.8, cy - d - 0.5, 1.3, 1.3], gold_color);
                    event
                        .batch
                        .fill([cx - d - 0.5, cy + d - 0.8, 1.3, 1.3], gold_color);
                    event
                        .batch
                        .fill([cx + d - 0.8, cy + d - 0.8, 1.3, 1.3], gold_color);
                }
            }
        }

        // Configurable western brass ammo counter (`8 / 8` + optional `.44-40` reserve with mini icon).
        let (rounds_now, reloading) = {
            let anim = client_anim().lock().unwrap();
            (
                anim.known_rounds,
                CLIENT_RELOADING.load(Ordering::Relaxed),
            )
        };
        let label = if reloading {
            format!("{rounds_now}/{TUBE_CAPACITY}+")
        } else {
            format!("{rounds_now} / {TUBE_CAPACITY}")
        };
        let mag_tw = event.batch.text_width(&label);

        let show_res = cfg.hud_show_reserve;
        let reserve = LOCAL_RESERVE_AMMO.load(Ordering::Relaxed);
        let res_label = if show_res {
            if reserve == u32::MAX {
                "∞".to_string()
            } else {
                reserve.min(9999).to_string()
            }
        } else {
            String::new()
        };
        let res_tw = if show_res {
            event.batch.text_width(&res_label)
        } else {
            0.0
        };

        let left_w = (mag_tw + 12.0).max(46.0);
        let right_w = if show_res {
            (13.0 + res_tw + 6.0).max(26.0)
        } else {
            0.0
        };
        let box_w = if show_res {
            left_w + 1.0 + right_w
        } else {
            52.0_f32.max(left_w)
        };
        let box_h = 15.0_f32;
        let Some([x0, y0]) = cfg.hud_position.layout(wf, hf, box_w, box_h) else {
            return;
        };

        let accent = cfg.hud_accent_color & 0xFF_FFFF;
        let border = if reloading {
            0xDD00_0000 | accent
        } else if rounds_now == 0 {
            0xCC_C9_4A_38
        } else {
            0xAE00_0000 | accent
        };
        event.batch.fill([x0, y0, box_w, box_h], 0x92_14_11_0E);
        event.batch.fill([x0, y0, box_w, 1.0], border);
        event.batch.fill([x0, y0 + box_h - 1.0, box_w, 1.0], border);

        let text_color = if rounds_now == 0 && !reloading {
            0xFF_FF_6E_59
        } else {
            0xFF_F5_E6_C8
        };
        let mag_area_w = if show_res { left_w } else { box_w };
        event.batch.text(
            &label,
            [x0 + (mag_area_w - mag_tw) * 0.5, y0 + 4.0],
            text_color,
            true,
        );

        if show_res {
            let div_x = x0 + left_w;
            event
                .batch
                .fill([div_x, y0 + 2.5, 1.0, box_h - 5.0], 0x6600_0000 | accent);
            draw_hud_mini_cartridge(event, div_x + 5.0, y0 + 3.0);
            let res_color = if reserve == u32::MAX {
                0xFF_FF_E0_82
            } else if reserve == 0 {
                0xFF_D9_6B_59
            } else {
                0xFF_D8_CB_B2
            };
            event
                .batch
                .text(&res_label, [div_x + 12.0, y0 + 4.0], res_color, true);
        }
    }
}
