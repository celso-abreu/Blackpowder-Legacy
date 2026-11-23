//! Just a Rifle — Winchester 1873 lever-action rifle for Horizon (wasm mod, v0.9.0).
//!
//! Clean-room Rust `v0.9.0` modular architecture:
//! - `constants` — Content IDs, ballistics, and exact Blockbench `[0..16]` texel pivots.
//! - `settings`  — `Options > Mods > Config` 29-setting registry & per-channel audio mixer.
//! - `skin`      — Mojang Base64 + UTF-16LE SHA-1 skin decoder & 18-skin UUID fallback.
//! - `arms`      — 1st-person `slim` (`3×12×4`) / `wide` (`4×12×4`) arm & 3D sleeve meshes.
//! - `pose`      — 2nd-order springs, TAC state machine, dual-arm poses & sight kinematics.
//! - `world_fx`  — Reactive world targets, per-material impact audio, bullet decals & casings.
//! - `server`    — Authoritative firing, stance spread, muzzle VFX & per-shell reload loop.

mod arms;
mod constants;
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
    ArmPose, BlockKey, CalculatePlayerTurnEvent, CameraPoseEvent, ClientHost,
    ClientResourcesReloadedEvent, ClientSetupEvent, ClientTickPreEvent, ComputeFovEvent, Flow,
    FluidKey, GetArmPoseEvent, GuiDraw, GuiLayerPreEvent, GuiRenderPostEvent, Hand,
    HearsProjectileImpactsEvent, Hooks, Host, HostSide, HumanoidPart, IPayloadContext, Identifier,
    ItemKey, KeyConflictContext, KeyMappingPreEvent, KeyRegistration, LevelRef, LevelTickPostEvent,
    ModelBakingCompletedEvent, ParticleKey, PayloadId, PlayLevelSoundAtPositionEvent,
    ProjectileImpactEvent, Registrar, RegistrationContext, RenderFramePreEvent, RenderHandEvent,
    RenderPlayerPostEvent, ServerHost, ServerTickEvent, ServerTickPhase, SettingChangedEvent,
    SoundPlay, SoundSource, StandaloneModelsEvent, SubmitCustomGeometryEvent, ToClient, ToServer,
    VarInt,
};

use crate::arms::PlayerArms;
use crate::constants::*;
use crate::pose::{
    build_fire_input, cam_recoil_curve, compute_gun_texel_view_pos, draw_gun_space_arm,
    draw_rifle_assembly, evaluate_weapon_pose, local_offhand_rifle_status, recoil_envelope,
    tex_to_unit, ActionAnim, ClientAnimState, RifleModels, SlungRifleState,
};
use crate::server::{
    on_rifle_input, rounds, tick_active_bullets, tick_reload_sessions, Winchester,
};
use crate::settings::{
    classify_incoming_level_sound, client_settings, default_channel_for_local_sound,
    mark_settings_dirty, mix_client_sound, refresh_client_settings, refresh_settings_if_dirty,
    register_all_settings, AdsMode, BackSlingStyle, LevelSoundDecision, SoundChannel,
};
use crate::skin::ResolvedSkin;
use crate::world_fx::{
    handle_bullet_terrain_impact, on_bullet_impact_sync, spawn_spent_casing_from_camera,
    step_world_effects, submit_world_effects,
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

pub(crate) static CLIENT_TICKS: AtomicU64 = AtomicU64::new(0);
pub(crate) static AIM_LAST_TICK: AtomicU64 = AtomicU64::new(0);
static LAST_AUTO_RELOAD_TICK: AtomicU64 = AtomicU64::new(0);
static FRAME_SEQ: AtomicU64 = AtomicU64::new(0);
static RIFLE_SEEN_FRAME: AtomicU64 = AtomicU64::new(0);
static ADS_BITS: AtomicU32 = AtomicU32::new(0);
static HITMARKER_TIMER_BITS: AtomicU32 = AtomicU32::new(0);
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
pub(crate) static RIFLE_KEY: OnceLock<ItemKey> = OnceLock::new();
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
    Some(RifleModels {
        body: assets.model(&Identifier::parse(MODEL_BODY_ID).ok()?).ok()?,
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

pub(crate) fn play_local_foley_channel(
    host: &dyn ClientHost,
    channel: SoundChannel,
    event_id: &str,
    volume: f32,
    pitch: f32,
) {
    let cfg = client_settings();
    let Some((vol, pit)) = mix_client_sound(&cfg, channel, volume, pitch) else {
        return;
    };
    let mut sound = SoundPlay::ui(event_id).with_volume(vol).with_pitch(pit);
    sound.source = SoundSource::Players;
    let _ = host.play_local_sound(&sound, 0);
}

pub(crate) fn play_local_foley(host: &dyn ClientHost, event_id: &str, volume: f32, pitch: f32) {
    play_local_foley_channel(
        host,
        default_channel_for_local_sound(event_id),
        event_id,
        volume,
        pitch,
    );
}

fn on_rifle_sync(value: RifleSync, cx: &mut dyn IPayloadContext) -> Result<(), String> {
    let cfg = client_settings();
    if value.kind.0 == SYNC_HIT_CONFIRM {
        HITMARKER_TIMER_BITS.store(0.14_f32.to_bits(), Ordering::Relaxed);
        if let Some((snd, vol, pitch)) = cfg.hit_confirm_sound.sound_spec()
            && let Ok(host) = cx.client_host()
        {
            play_local_foley_channel(host, SoundChannel::Feedback, snd, vol, pitch);
        }
        return Ok(());
    }

    let rounds_now = (value.rounds.0.max(0) as u32).min(TUBE_CAPACITY);
    let mut foley: Option<(SoundChannel, &'static str, f32, f32)> = None;

    {
        let mut anim = client_anim().lock().unwrap();
        anim.known_rounds = rounds_now;
        match value.kind.0 {
            SYNC_SHOT => {
                CLIENT_RELOADING.store(false, Ordering::Relaxed);
                anim.on_shot(rounds_now);
            }
            SYNC_DRY_CLICK => {
                anim.on_dry_click(rounds_now);
                if cfg.dry_click_alert {
                    foley = Some((SoundChannel::Feedback, SND_COCK, 0.72, 1.32));
                }
                if rounds_now == 0 && cfg.auto_reload {
                    PENDING_AUTO_RELOAD.store(true, Ordering::Relaxed);
                }
            }
            SYNC_WET_MISFIRE => {
                CLIENT_RELOADING.store(false, Ordering::Relaxed);
                anim.on_dry_click(rounds_now);
                foley = Some((SoundChannel::Mechanism, SND_MISFIRE_WET, 0.76, 0.92));
            }
            SYNC_RELOAD_START => {
                CLIENT_RELOADING.store(true, Ordering::Relaxed);
                AIMING.store(false, Ordering::Relaxed);
                TOGGLE_AIM_LATCH.store(false, Ordering::Relaxed);
                anim.action = ActionAnim::ReloadIntro { elapsed: 0.0 };
                foley = Some((SoundChannel::Reload, SND_RELOAD_INTRO, 0.90, 1.0));
            }
            SYNC_SHELL_INSERTED => {
                CLIENT_RELOADING.store(true, Ordering::Relaxed);
                anim.action = ActionAnim::ReloadInsert { elapsed: 0.0 };
                foley = Some((SoundChannel::Reload, SND_RELOAD_INSERT, 0.92, 1.0));
            }
            SYNC_RELOAD_END_NORM => {
                CLIENT_RELOADING.store(false, Ordering::Relaxed);
                anim.action = ActionAnim::ReloadEndNorm { elapsed: 0.0 };
                foley = Some((SoundChannel::Reload, SND_RELOAD_END, 0.88, 1.0));
            }
            SYNC_RELOAD_END_EMPTY => {
                CLIENT_RELOADING.store(false, Ordering::Relaxed);
                anim.action = ActionAnim::ReloadEndEmpty {
                    elapsed: 0.0,
                    lever_sound_played: false,
                    cock_sound_played: false,
                };
                foley = Some((SoundChannel::Reload, SND_RELOAD_END, 0.85, 1.0));
            }
            SYNC_RELOAD_ABORTED => {
                CLIENT_RELOADING.store(false, Ordering::Relaxed);
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

    if let Some((ch, snd, vol, pitch)) = foley
        && let Ok(host) = cx.client_host()
    {
        play_local_foley_channel(host, ch, snd, vol, pitch);
    }
    Ok(())
}

fn on_rifle_skin_sync(value: RifleSkinSync, _: &mut dyn IPayloadContext) -> Result<(), String> {
    let mut anim = client_anim().lock().unwrap();
    anim.skin = ResolvedSkin {
        primary: value.sprite,
        fallback: value.fallback,
        slim: value.slim.0 != 0,
    };
    Ok(())
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
        let _ = RIFLE_INPUT.set(input);
        let _ = RIFLE_SYNC.set(sync);
        let _ = RIFLE_SKIN_SYNC.set(skin);
        let _ = BULLET_IMPACT_SYNC.set(impact);
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
            } else if event.owner != 0
                && event.entity != Some(event.owner)
                && let Some(sync) = RIFLE_SYNC.get()
                && let Some(level) = host.level(event.dimension)
                && let Some(players) = level.online_players(host)
                && let Some(shooter) = players
                    .into_iter()
                    .find(|p| p.entity().id() == event.owner)
            {
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
        Flow::Continue
    }

    fn setting_changed(&self, host: &mut dyn ClientHost, _: &mut SettingChangedEvent<'_>) {
        refresh_client_settings(host);
        if client_settings().ads_mode == AdsMode::Hold
            && TOGGLE_AIM_LATCH.swap(false, Ordering::Relaxed)
        {
            AIMING.store(false, Ordering::Relaxed);
            let _ = host.set_mapping_down("key.use", false);
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
                let allow_fire = {
                    let mut anim = client_anim().lock().unwrap();
                    match anim.action {
                        ActionAnim::Draw { .. }
                        | ActionAnim::FireCycle { .. }
                        | ActionAnim::ReloadEndNorm { .. }
                        | ActionAnim::ReloadEndEmpty { .. } => false,
                        ActionAnim::Inspect { .. } => {
                            anim.action = ActionAnim::None;
                            true
                        }
                        _ => true,
                    }
                };
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
            PENDING_FIRE.store(false, Ordering::Relaxed);
            PENDING_AUTO_RELOAD.store(false, Ordering::Relaxed);
            TOGGLE_AIM_LATCH.store(false, Ordering::Relaxed);
            mark_settings_dirty();
            CLIENT_SPRINTING.store(false, Ordering::Relaxed);
            CLIENT_SNEAKING.store(false, Ordering::Relaxed);
            CLIENT_RELOADING.store(false, Ordering::Relaxed);
            SLUNG_RIFLES.lock().unwrap().clear();
            let mut anim = client_anim().lock().unwrap();
            anim.was_holding = false;
            anim.was_slung = false;
            anim.action = ActionAnim::None;
            anim.skin_requested = false;
            return;
        }

        let holding = holding_rifle_recently() || AIMING.load(Ordering::Relaxed);
        let is_slung = local_offhand_rifle_status(host).is_some();

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
        let mut foley_to_play: Option<(&'static str, f32, f32)> = None;

        {
            let mut anim = client_anim().lock().unwrap();
            if holding && !anim.skin_requested {
                anim.skin_requested = true;
                want_skin_request = true;
            }

            if holding && !anim.was_holding {
                anim.was_holding = true;
                anim.action = ActionAnim::Draw { elapsed: 0.0 };
                foley_to_play = Some((SND_DRAW, 0.85, 1.0));
            } else if !holding && anim.was_holding {
                anim.was_holding = false;
                anim.action = ActionAnim::None;
            }

            if is_slung && !anim.was_slung {
                anim.was_slung = true;
                if foley_to_play.is_none() {
                    // Crisp leather/wood sling foley when pressing `F` to stow onto the back.
                    foley_to_play = Some((SND_DRAW, 0.72, 0.84));
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

        if let Some((snd, vol, pitch)) = foley_to_play {
            play_local_foley(host, snd, vol, pitch);
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
            let mut anim = client_anim().lock().unwrap();
            if matches!(
                anim.action,
                ActionAnim::Inspect { .. }
                    | ActionAnim::ReloadIntro { .. }
                    | ActionAnim::ReloadInsert { .. }
            ) {
                anim.action = ActionAnim::None;
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
            let mut anim = client_anim().lock().unwrap();
            if matches!(anim.action, ActionAnim::Inspect { .. }) {
                anim.action = ActionAnim::None;
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
                play_local_foley(host, SND_INSPECT, 0.90, 1.0);
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
        tick_reload_sessions(host, now);
        tick_active_bullets(host, now);
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
                        if !lever_sound_played && next >= LEVER_DELAY_SECS {
                            lever_sound_played = true;
                            triggered_foley.push((SoundChannel::Mechanism, SND_LEVER, 0.86, 1.0));
                        }
                        if !casing_ejected && next >= CASING_HANDOFF_SECS {
                            casing_ejected = true;
                            eject_casing_aim = Some(aim_y.clamp(0.0, 1.0));
                        }
                        if !cock_sound_played && next >= LEVER_DELAY_SECS + 0.24 {
                            cock_sound_played = true;
                            anim.hammer_uncocked = false;
                            triggered_foley.push((SoundChannel::Mechanism, SND_COCK, 0.72, 1.04));
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
                    if !lever_sound_played && next >= 0.15 {
                        lever_sound_played = true;
                        triggered_foley.push((SoundChannel::Reload, SND_LEVER, 0.90, 0.98));
                    }
                    if !cock_sound_played && next >= 0.40 {
                        cock_sound_played = true;
                        anim.hammer_uncocked = false;
                        triggered_foley.push((SoundChannel::Reload, SND_COCK, 0.78, 1.0));
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
            play_local_foley_channel(host, ch, snd, vol, pitch);
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

    fn get_arm_pose(&self, _: &dyn ClientHost, event: &mut GetArmPoseEvent<'_>) -> Option<ArmPose> {
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
        None
    }

    fn render_player_post(&self, host: &dyn ClientHost, event: &mut RenderPlayerPostEvent<'_>) {
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
            let sling_style = client_settings().back_sling_style;
            if sling_style == BackSlingStyle::Hidden {
                return;
            }
            if let Some((off_rounds, chest_armored)) = local_offhand_rifle_status(host) {
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
            } else {
                return;
            }
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
        let _ = draw_rifle_assembly(draw, models, 0.0, hammer_deg, 0.0, 0.0);
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

        // Optional subtle 4-tick brass hitmarker around screen center on confirmed entity hits.
        let hit_rem = f32::from_bits(HITMARKER_TIMER_BITS.load(Ordering::Relaxed));
        if cfg.hud_hitmarker && hit_rem > 0.001 {
            let alpha = ((hit_rem / 0.14).clamp(0.0, 1.0) * 225.0) as u32;
            let hm_color = (alpha << 24) | (cfg.hud_accent_color & 0xFF_FFFF);
            let cx = wf * 0.5;
            let cy = hf * 0.5;
            for step in 2..=5 {
                let d = step as f32;
                event.batch.fill([cx - d - 0.5, cy - d - 0.5, 1.2, 1.2], hm_color);
                event.batch.fill([cx + d - 0.7, cy - d - 0.5, 1.2, 1.2], hm_color);
                event.batch.fill([cx - d - 0.5, cy + d - 0.7, 1.2, 1.2], hm_color);
                event.batch.fill([cx + d - 0.7, cy + d - 0.7, 1.2, 1.2], hm_color);
            }
        }

        // Configurable western brass ammo counter (`8 / 8`), tintable via `hud.accent_color`.
        let box_w = 52.0_f32;
        let box_h = 15.0_f32;
        let Some([x0, y0]) = cfg.hud_position.layout(wf, hf, box_w, box_h) else {
            return;
        };
        let (rounds_now, reloading) = {
            let anim = client_anim().lock().unwrap();
            (
                anim.known_rounds,
                CLIENT_RELOADING.load(Ordering::Relaxed),
            )
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

        let label = if reloading {
            format!("{rounds_now}/{TUBE_CAPACITY}+")
        } else {
            format!("{rounds_now} / {TUBE_CAPACITY}")
        };
        let tw = event.batch.text_width(&label);
        let text_color = if rounds_now == 0 && !reloading {
            0xFF_FF_6E_59
        } else {
            0xFF_F5_E6_C8
        };
        event
            .batch
            .text(&label, [x0 + (box_w - tw) * 0.5, y0 + 4.0], text_color, true);
    }
}
