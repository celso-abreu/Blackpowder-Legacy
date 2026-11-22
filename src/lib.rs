//! Blackpowder Legacy — Winchester 1873 lever-action rifle (wasm mod, v0.9.0).
//!
//! Clean-room Rust `v0.9.0` architecture (inspired by TAC `0.3.10.1` design & timings):
//! - **Multi-part articulated 1st-person model (`models/special/winchester_*.json`):**
//!   `standalone_models` registers 6 sub-models (`body`, `lever`, `hammer`, `trigger`,
//!   `gate`, `cartridge`) resolved in `model_baking_completed` via `host.render_assets().model`.
//!   In `render_hand`, vanilla's static 1-piece item render is replaced (`Flow::Cancel`) by
//!   all 6 articulated parts + both 1st-person player arms inside one unified `[0..1]` rifle
//!   coordinate frame (`1` unit = `16` JSON texels), keeping hands and moving parts locked
//!   together through ADS, 2nd-order recoil, lever cycling, per-round gate loading, and inspect.
//! - **Real player skin + `slim`/`wide` 1st-person arms with 3D sleeves:**
//!   Immediate fallback resolves `default_skin_for_uuid(uuid)` (`18` modern skins: `0..9`
//!   slim `3x12x4`, `9..18` wide `4x12x4`). Online custom skins are resolved on the server
//!   via `ServerHost::profile_by_uuid`, decoding Base64 `"textures"` into Horizon's
//!   `"skins/" + sha1_utf16le(hash)` sprite key + `"metadata"."model" == "slim"`, synced
//!   once over `RifleSkinSync`. Both arms use `[1.92, 2.12, 1.92]` gun-space scale
//!   (`[0.864, 0.954, 0.864]` world blocks) so 1st-person hand size matches the player's
//!   vanilla empty hand while wrapping naturally underneath the stock wrist and forestock.
//! - **TAC-style keyframe state machine & bundled CC0 `.ogg` audio (`sounds.json`):**
//!   Covers `Draw`, `Idle`/`Walk`/`Sprint`/`ADS`, `FireCycle` (trigger pull, hammer fall,
//!   lever throw `+48°`, hammer re-cock), `DryClick`, `ReloadIntro`, per-shell `ReloadLoop`
//!   (brass `.44-40` cartridge sliding through the depressed `-23°` King's loading gate),
//!   `ReloadEndNorm`, `ReloadEndEmpty` (full lever chambering on empty reload), and
//!   `Inspect` (`I` key, remappable).
//! - **Diagonal Back-Sling when swapped to Off-Hand (`F` key):**
//!   Pressing `F` to move the Winchester into the off-hand (left hand) stows it diagonally
//!   (`44°`) across the player's back (`Hooks::render_player_post` locked to `HumanoidPart::Body`
//!   through standing, walking, torso twist, and sneaking), relaxes the off-hand arm to
//!   `ArmPose::Empty` (`Hooks::get_arm_pose`), suppresses duplicate left-hand rendering in
//!   1st/3rd person, and blocks accidental off-hand ADS right-clicks (`ItemPart::use_item`).

use mod_api::{
    ArmPose, BlockEntityDrawContext, CalculatePlayerTurnEvent, CameraPoseEvent, CameraType,
    ClientHost, ClientResourcesReloadedEvent, ClientSetupEvent, ClientTickPreEvent,
    ComputeFovEvent, CubeDeformation, CubeListBuilder, EntityUuid, EquipmentSlot, Flow,
    GetArmPoseEvent, GuiDraw, GuiLayerPreEvent, GuiRenderPostEvent, Hand,
    HearsProjectileImpactsEvent, Hooks, Host, HostSide, HumanoidPart, IPayloadContext, Identifier,
    InteractionResult, ItemBarEvent, ItemKey, ItemPart, ItemStack, KeyConflictContext,
    KeyMappingPreEvent, KeyRegistration, LayerDefinition, LevelTickPostEvent, LightCoords,
    LightLayer, LivingRef, MeshDefinition, ModelBakingCompletedEvent, ModelInstance, ModelPart,
    ModelRef, OnUseTickEvent, Overlay, PartPose, ParticleKey, ParticleOptions, PayloadId,
    ProjectileImpactEvent, Registrar, RegistrationContext, ReleaseUsingEvent, RenderFramePreEvent,
    RenderHandEvent, RenderPlayerPostEvent, ServerHost, ServerTickEvent, ServerTickPhase,
    ShouldCauseReequipAnimationEvent, SoundPlay, SoundSource, StandaloneModelsEvent,
    SubmitCustomGeometryEvent, ToClient, ToServer, UseDurationEvent, UseItemEvent, VarInt,
    WorldDrawContext,
};
use std::collections::BTreeMap;
use std::f32::consts::{PI, TAU};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

// ---------------------------------------------------------------------------
// Content & Asset Identifiers
// ---------------------------------------------------------------------------

const MOD_ID: &str = "blackpowder_legacy";
const WINCHESTER: &str = "blackpowder_legacy:winchester_1873";
const CARTRIDGE: &str = "blackpowder_legacy:cartridge_44";
const BULLET: &str = "blackpowder_legacy:bullet";

const MODEL_BODY_ID: &str = "blackpowder_legacy:special/winchester_body";
const MODEL_LEVER_ID: &str = "blackpowder_legacy:special/winchester_lever";
const MODEL_HAMMER_ID: &str = "blackpowder_legacy:special/winchester_hammer";
const MODEL_TRIGGER_ID: &str = "blackpowder_legacy:special/winchester_trigger";
const MODEL_GATE_ID: &str = "blackpowder_legacy:special/winchester_gate";
const MODEL_CARTRIDGE_ID: &str = "blackpowder_legacy:special/winchester_cartridge";
const MODEL_CASING_ID: &str = "blackpowder_legacy:special/winchester_casing";
const MODEL_BULLET_HOLE_ID: &str = "blackpowder_legacy:special/bullet_hole";

const SND_FIRE: &str = "blackpowder_legacy:item.winchester_fire";
const SND_THUMP: &str = "blackpowder_legacy:item.winchester_thump";
const SND_LEVER: &str = "blackpowder_legacy:item.winchester_lever";
const SND_COCK: &str = "blackpowder_legacy:item.winchester_cock";
const SND_RELOAD_INTRO: &str = "blackpowder_legacy:animation.winchester_reload_intro";
const SND_RELOAD_INSERT: &str = "blackpowder_legacy:animation.winchester_reload_insert";
const SND_RELOAD_END: &str = "blackpowder_legacy:animation.winchester_reload_end";
const SND_DRAW: &str = "blackpowder_legacy:animation.winchester_draw";
const SND_INSPECT: &str = "blackpowder_legacy:animation.winchester_inspect";
const SND_SHELL_DROP: &str = "blackpowder_legacy:item.winchester_shell_drop";
const SND_IMPACT_HIT: &str = "minecraft:block.stone.hit";

const RELOAD_MAPPING: &str = "reload";
const INSPECT_MAPPING: &str = "inspect";
const ATTACK_MAPPING: &str = "key.attack";

const INPUT_CHANNEL: &str = "blackpowder_legacy:rifle_input";
const SYNC_CHANNEL: &str = "blackpowder_legacy:rifle_sync";
const SKIN_CHANNEL: &str = "blackpowder_legacy:rifle_skin";
const IMPACT_CHANNEL: &str = "blackpowder_legacy:bullet_impact";

const SMOKE_PARTICLE: &str = "minecraft:smoke";
const WHITE_SMOKE_PARTICLE: &str = "minecraft:white_smoke";
const FLAME_PARTICLE: &str = "minecraft:small_flame";
const POOF_PARTICLE: &str = "minecraft:poof";
const BLOCK_PARTICLE: &str = "minecraft:block";
const DEFAULT_DIMENSION: &str = "minecraft:overworld";
const MAX_BULLET_HOLES: usize = 96;
const BULLET_HOLE_LIFETIME_SECS: f32 = 35.0;
const BULLET_HOLE_BASE_SCALE: f32 = 0.44;
const MAX_SPENT_CASINGS: usize = 48;
const CASING_LIFETIME_SECS: f32 = 20.0;
const CASING_BASE_SCALE: f32 = 0.64;
const CASING_REST_RADIUS: f64 = 0.012;
const CASING_RISE_START_SECS: f32 = LEVER_DELAY_SECS + 0.06;
const CASING_HANDOFF_SECS: f32 = LEVER_DELAY_SECS + 0.16;
const CASING_LAST_RISE_START_SECS: f32 = 0.06;
const CASING_LAST_HANDOFF_SECS: f32 = 0.15;
const AMMO_BAR_RGB: u32 = 0xC9_A2_27;
const DEFAULT_STEVE_WIDE: &str = "entity/player/wide/steve";

// ---------------------------------------------------------------------------
// Weapon Balance & Timing Constants (20 TPS)
// ---------------------------------------------------------------------------

const TUBE_CAPACITY: u32 = 8;
const USE_DURATION: i32 = 72_000;

/// Ticks between shots: recoil (`3t`) + lever throw & lockup (`10t`) = `13t` (`0.65 s`).
const LEVER_TICKS: u64 = 13;
const LEVER_DELAY_SECS: f32 = 0.14;
const LEVER_DURATION_SECS: f32 = 0.48;

/// Ticks holding right-click before full server-side ADS accuracy settles.
const ADS_SETTLE_TICKS: i32 = 4;

// Per-cartridge server reload timing (at 20 TPS).
const RELOAD_OPEN_TICKS: u64 = 8;
const RELOAD_INSERT_TICKS: u64 = 10;
const RELOAD_CLOSE_NORM_TICKS: u64 = 7;
const RELOAD_CLOSE_EMPTY_TICKS: u64 = 15;

// Draw & inspect durations in seconds.
const DRAW_DURATION_SECS: f32 = 0.48;
const INSPECT_DURATION_SECS: f32 = 3.40;

// Winchester 1873 (.44-40) projectile speed & stance-dependent inaccuracy.
const BULLET_SPEED: f32 = 15.0;
const SPREAD_ADS_CROUCH: f32 = 0.04;
const SPREAD_ADS_STAND: f32 = 0.06;
const SPREAD_ADS_MOVING: f32 = 0.12;
const SPREAD_HIP_CROUCH: f32 = 1.10;
const SPREAD_HIP_STAND: f32 = 1.50;
const SPREAD_HIP_MOVING: f32 = 2.20;

/// World FOV scale at full ADS (`~1.5x` magnification, TAC iron-sight baseline).
const AIM_FOV: f32 = 0.67;
/// Vignette darkness at full aim (`1.0` is invisible).
const AIM_VIGNETTE: f32 = 0.68;

// ---------------------------------------------------------------------------
// Exact Model-Space Coordinates (`winchester_1873.bbmodel` / `item/winchester_1873.json`)
// ---------------------------------------------------------------------------

/// `ItemInHandRenderer` right-hand base translation (`equip = 0`, `attack = 0`).
const HAND_BASE: [f32; 3] = [0.56, -0.52, -0.72];

/// First-person right-hand offset placing the cocked Winchester 1873 iron sights
/// (`massa_lamina` over `alca_entalhe_base` at baked `X = 8.139`, `Y = 12.684`) dead-center on screen
/// at `RIFLE_DISPLAY_SCALE = 0.60`.
const ADS_OFFSET: [f32; 3] = [-0.5277, 0.2944, 0.04];

/// Mirror of `display.firstperson_righthand` in `models/item/winchester_1873.json` (`+33.3%`
/// larger than `0.45` so the receiver, barrel, and walnut stock feel substantial in the hands
/// while keeping buttstock `Z <= -0.194` well clear of the `-0.05` near plane).
const RIFLE_DISPLAY_TRANS: [f32; 3] = [-0.6 / 16.0, 0.8 / 16.0, -6.4 / 16.0];
const RIFLE_DISPLAY_SCALE: f32 = 0.60;

/// Muzzle crown (`boca_cano` center) in `[0..16]` JSON texel coordinates.
const MUZZLE_TEXELS: [f32; 3] = [8.139, 11.68, -16.0];

/// Top receiver carrier / ejection port (`trava_dorsal`: `X=[7.803..8.475]`,
/// `Y=[12.032..12.224]`, `Z=[10.400..11.456]`) in `[0..16]` JSON texels:
/// - `EJECTOR_START_TEXELS`: inside the carrier slot beneath `trava_dorsal`.
/// - `EJECTOR_HANDOFF_TEXELS`: just clearing the top-right lip of `trava_dorsal` where
///   the 1st-person shell hands off seamlessly to world-space physics.
const EJECTOR_START_TEXELS: [f32; 3] = [8.139, 11.42, 10.93];
const EJECTOR_HANDOFF_TEXELS: [f32; 3] = [8.42, 12.78, 11.05];

/// Sub-model pivots in `[0..16]` JSON texel coordinates (`BB -> JM`: `8 + x`, `y`, `8 + z`).
const LEVER_PIVOT_TEX: [f32; 3] = [8.139, 8.912, 13.376];
const HAMMER_PIVOT_TEX: [f32; 3] = [8.139, 11.072, 15.488];
const TRIGGER_PIVOT_TEX: [f32; 3] = [8.139, 9.104, 14.768];
const GATE_PIVOT_TEX: [f32; 3] = [8.936, 10.592, 10.432];
const RECOIL_PIVOT_TEXELS: [f32; 3] = [8.14, 8.60, 22.00];

/// Hand grip anchors in `[0..16]` JSON texels (wrapping underneath `punho` stock wrist
/// and cradling snugly around `guarda_mao_traseiro` `[7.56..8.72, 9.86..11.74, -0.64..7.52]`
/// with zero air gap while staying well below the `12.68` iron-sight line).
const GRIP_WRIST_TEXELS: [f32; 3] = [9.55, 5.45, 16.50];
const FORESTOCK_TEXELS: [f32; 3] = [8.55, 8.05, 3.60];

/// First-person player arm scale compensated for `RIFLE_DISPLAY_SCALE` (`0.60`):
/// - Cross-section `1.36 * 0.60 = 0.816` world blocks and forearm reach `1.54 * 0.60 = 0.924`
///   world blocks so the player's hands stay naturally sized while the rifle sits `+33%` larger
///   inside them (and both elbows remain `<= -0.23` behind the `-0.05` near plane).
/// - Both right and left arms share identical world thickness and `3x12x4` (`slim`) / `4x12x4`
///   (`wide`) geometry with `+0.25` 3D outer sleeves.
const ARM_GUN_SPACE_WIDTH: f32 = 1.36;
const ARM_GUN_SPACE_LENGTH: f32 = 1.54;

/// Third-person diagonal back-sling constants (`Hooks::render_player_post` on `HumanoidPart::Body`):
/// - `AVATAR_RENDER_SCALE` (`15/16 = 0.9375`) and `AVATAR_MODEL_OFFSET_Y` (`-1.501`) match
///   `AvatarRenderer::scale` / `base_transform` so the slung rifle stays welded to the player's
///   spine across standing, walking, torso twist, and sneaking (`Body` `x_rot = 0.5 rad`).
/// - `THIRDPERSON_RIFLE_SCALE` (`0.55`) matches `thirdperson_righthand` / `thirdperson_lefthand`
///   so swapping with `F` preserves exact 3rd-person scale.
/// - `BACK_SLING_DIAG_DEG = -44.0` angles the barrel upward over the main-hand shoulder while
///   the walnut buttstock rests down near the off-hand hip, with the flat steel receiver side
///   flush against the back and the King's loading gate facing outward.
const AVATAR_RENDER_SCALE: f32 = 0.9375;
const AVATAR_MODEL_OFFSET_Y: f32 = -1.501;
const THIRDPERSON_RIFLE_SCALE: f32 = 0.55;
const BACK_SLING_MODEL_SCALE: f32 = THIRDPERSON_RIFLE_SCALE / AVATAR_RENDER_SCALE;
const BACK_SLING_DIAG_DEG: f32 = -44.0;
const BACK_SLING_CENTER_TEX: [f32; 3] = [8.139, 8.50, 8.20];
const BACK_SLING_OFFSET_Y_TEX: f32 = -5.80;
const BACK_SLING_JACKET_Z_TEX: f32 = 3.10;
const BACK_SLING_ARMOR_Z_TEX: f32 = 3.85;
const BACK_SLING_REMOTE_Z_TEX: f32 = 3.55;

/// Modern 18-skin default table (`crates/horizon-client-render/src/skins.rs`).
const DEFAULT_SKINS: [(&str, bool); 18] = [
    ("entity/player/slim/alex", true),
    ("entity/player/slim/ari", true),
    ("entity/player/slim/efe", true),
    ("entity/player/slim/kai", true),
    ("entity/player/slim/makena", true),
    ("entity/player/slim/noor", true),
    ("entity/player/slim/steve", true),
    ("entity/player/slim/sunny", true),
    ("entity/player/slim/zuri", true),
    ("entity/player/wide/alex", false),
    ("entity/player/wide/ari", false),
    ("entity/player/wide/efe", false),
    ("entity/player/wide/kai", false),
    ("entity/player/wide/makena", false),
    ("entity/player/wide/noor", false),
    ("entity/player/wide/steve", false),
    ("entity/player/wide/sunny", false),
    ("entity/player/wide/zuri", false),
];

// ---------------------------------------------------------------------------
// Network Payloads
// ---------------------------------------------------------------------------

const ACTION_FIRE: i32 = 1;
const ACTION_RELOAD: i32 = 2;
const ACTION_RELOAD_CANCEL: i32 = 3;
const ACTION_SKIN_REQUEST: i32 = 4;

const SYNC_SHOT: i32 = 1;
const SYNC_DRY_CLICK: i32 = 2;
const SYNC_RELOAD_START: i32 = 3;
const SYNC_SHELL_INSERTED: i32 = 4;
const SYNC_RELOAD_END_NORM: i32 = 5;
const SYNC_RELOAD_END_EMPTY: i32 = 6;
const SYNC_RELOAD_ABORTED: i32 = 7;

#[derive(horizon_mod_api_macros::PayloadCodec)]
struct RifleInput {
    action: VarInt,
    muzzle_right_mm: VarInt,
    muzzle_up_mm: VarInt,
    muzzle_fwd_mm: VarInt,
    aim_pct: VarInt,
}

impl RifleInput {
    const fn simple(action: i32) -> Self {
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
struct RifleSync {
    kind: VarInt,
    rounds: VarInt,
}

#[derive(horizon_mod_api_macros::PayloadCodec)]
struct RifleSkinSync {
    sprite: String,
    fallback: String,
    slim: VarInt,
}

#[derive(horizon_mod_api_macros::PayloadCodec)]
struct BulletImpactSync {
    dimension: String,
    block_x: VarInt,
    block_y: VarInt,
    block_z: VarInt,
    face_x: VarInt,
    face_y: VarInt,
    face_z: VarInt,
    rel_x_dm: VarInt,
    rel_y_dm: VarInt,
    rel_z_dm: VarInt,
}

// ---------------------------------------------------------------------------
// Static State & Spring-Damper Dynamics
// ---------------------------------------------------------------------------

static AIMING: AtomicBool = AtomicBool::new(false);
static CLIENT_RELOADING: AtomicBool = AtomicBool::new(false);
static CLIENT_SPRINTING: AtomicBool = AtomicBool::new(false);
static CLIENT_SNEAKING: AtomicBool = AtomicBool::new(false);
static PENDING_FIRE: AtomicBool = AtomicBool::new(false);
static PENDING_RELOAD_CANCEL: AtomicBool = AtomicBool::new(false);

static CLIENT_TICKS: AtomicU64 = AtomicU64::new(0);
static AIM_LAST_TICK: AtomicU64 = AtomicU64::new(0);
static FRAME_SEQ: AtomicU64 = AtomicU64::new(0);
static RIFLE_SEEN_FRAME: AtomicU64 = AtomicU64::new(0);
static ADS_BITS: AtomicU32 = AtomicU32::new(0);
static LAST_WORLD_FOV_BITS: AtomicU32 = AtomicU32::new(0x428C_0000); // 70.0_f32.to_bits()
static LAST_MUZZLE_VIEW_X: AtomicU32 = AtomicU32::new(0x3F07_1A9F); // 0.52775_f32
static LAST_MUZZLE_VIEW_Y: AtomicU32 = AtomicU32::new(0xBEAA_0000); // -0.33203_f32
static LAST_MUZZLE_VIEW_Z: AtomicU32 = AtomicU32::new(0xC001_47AE); // -2.02000_f32
static LAST_EJECTOR_VIEW_X: AtomicU32 = AtomicU32::new(0x3F09_AE14); // 0.5378_f32
static LAST_EJECTOR_VIEW_Y: AtomicU32 = AtomicU32::new(0xBE94_FDF4); // -0.2910_f32
static LAST_EJECTOR_VIEW_Z: AtomicU32 = AtomicU32::new(0xBF80_B852); // -1.0056_f32

static SERVER_TICKS: AtomicU64 = AtomicU64::new(0);
static AIM_TICKS: Mutex<BTreeMap<u128, (i32, u64)>> = Mutex::new(BTreeMap::new());
static LEVER_READY: Mutex<BTreeMap<u128, u64>> = Mutex::new(BTreeMap::new());
static RELOAD_SESSIONS: Mutex<BTreeMap<u128, ReloadSession>> = Mutex::new(BTreeMap::new());

static RIFLE_INPUT: OnceLock<ToServer<RifleInput>> = OnceLock::new();
static RIFLE_SYNC: OnceLock<ToClient<RifleSync>> = OnceLock::new();
static RIFLE_SKIN_SYNC: OnceLock<ToClient<RifleSkinSync>> = OnceLock::new();
static BULLET_IMPACT_SYNC: OnceLock<ToClient<BulletImpactSync>> = OnceLock::new();
static RIFLE_KEY: OnceLock<ItemKey> = OnceLock::new();
static SMOKE_KEY: OnceLock<ParticleKey> = OnceLock::new();
static WHITE_SMOKE_KEY: OnceLock<ParticleKey> = OnceLock::new();
static FLAME_KEY: OnceLock<ParticleKey> = OnceLock::new();
static POOF_KEY: OnceLock<ParticleKey> = OnceLock::new();
static BLOCK_PARTICLE_KEY: OnceLock<ParticleKey> = OnceLock::new();
static PLAYER_ARMS: OnceLock<PlayerArms> = OnceLock::new();
static RIFLE_MODELS: Mutex<Option<RifleModels>> = Mutex::new(None);
static CLIENT_ANIM: OnceLock<Mutex<ClientAnimState>> = OnceLock::new();
static WORLD_EFFECTS: OnceLock<Mutex<WorldEffectsState>> = OnceLock::new();
static SLUNG_RIFLES: Mutex<BTreeMap<u32, SlungRifleState>> = Mutex::new(BTreeMap::new());

#[derive(Clone, Copy)]
struct SlungRifleState {
    last_tick: u64,
    hammer_uncocked: bool,
    main_arm_left: bool,
}

#[derive(Clone, Copy)]
struct RifleModels {
    body: ModelRef,
    lever: ModelRef,
    hammer: ModelRef,
    trigger: ModelRef,
    gate: ModelRef,
    cartridge: ModelRef,
    casing: ModelRef,
    bullet_hole: ModelRef,
}

#[derive(Clone)]
struct ResolvedSkin {
    primary: String,
    fallback: String,
    slim: bool,
}

impl Default for ResolvedSkin {
    fn default() -> Self {
        Self {
            primary: DEFAULT_STEVE_WIDE.into(),
            fallback: DEFAULT_STEVE_WIDE.into(),
            slim: false,
        }
    }
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

#[derive(Clone, Copy, PartialEq)]
enum ActionAnim {
    None,
    Draw {
        elapsed: f32,
    },
    FireCycle {
        elapsed: f32,
        keep_uncocked: bool,
        lever_sound_played: bool,
        cock_sound_played: bool,
        casing_ejected: bool,
    },
    DryClick {
        elapsed: f32,
    },
    ReloadIntro {
        elapsed: f32,
    },
    ReloadInsert {
        elapsed: f32,
    },
    ReloadEndNorm {
        elapsed: f32,
    },
    ReloadEndEmpty {
        elapsed: f32,
        lever_sound_played: bool,
        cock_sound_played: bool,
    },
    Inspect {
        elapsed: f32,
    },
}

/// Semi-implicit second-order spring-damper (`f` Hz natural frequency, `zeta` damping,
/// `r` initial velocity response). Sub-stepped so arbitrary frame deltas remain stable.
#[derive(Clone, Copy)]
struct SecondOrder {
    k1: f32,
    k2: f32,
    k3: f32,
    xp: f32,
    y: f32,
    yd: f32,
}

impl SecondOrder {
    fn new(f: f32, zeta: f32, r: f32, x0: f32) -> Self {
        let w = 2.0 * PI * f;
        Self {
            k1: zeta / (PI * f),
            k2: 1.0 / (w * w),
            k3: r * zeta / w,
            xp: x0,
            y: x0,
            yd: 0.0,
        }
    }

    fn step(&mut self, dt: f32, x: f32) -> f32 {
        let dt = dt.clamp(0.0, 0.05);
        if dt <= 1e-6 {
            return self.y;
        }
        let steps = ((dt / 0.008).ceil() as u32).clamp(1, 8);
        let h = dt / (steps as f32);
        let xd = (x - self.xp) / dt;
        self.xp = x;
        let k2_stable = self
            .k2
            .max(0.5 * h * h + 0.5 * h * self.k1)
            .max(h * self.k1);
        for _ in 0..steps {
            self.y += h * self.yd;
            self.yd += h * (x + self.k3 * xd - self.y - self.k1 * self.yd) / k2_stable;
        }
        self.y
    }

    fn reset(&mut self, x0: f32) {
        self.xp = x0;
        self.y = x0;
        self.yd = 0.0;
    }
}

struct ClientAnimState {
    aim_dyn: SecondOrder,
    recoil_dyn: SecondOrder,
    sprint_dyn: SecondOrder,
    action: ActionAnim,
    was_holding: bool,
    was_slung: bool,
    known_rounds: u32,
    hammer_uncocked: bool,
    walk_phase: f32,
    walk_speed: f32,
    idle_time: f32,
    shot_ads_factor: f32,
    shot_crouch_factor: f32,
    shot_roll_sign: f32,
    recoil_target_timer: Option<f32>,
    cam_recoil_timer: Option<f32>,
    cam_pitch_kick: f32,
    cam_yaw_kick: f32,
    jolt_timer: f32,
    jolt_yaw_sign: f32,
    rng: u32,
    skin_requested: bool,
    skin: ResolvedSkin,
}

impl ClientAnimState {
    fn new() -> Self {
        Self {
            aim_dyn: SecondOrder::new(3.8, 0.88, 1.05, 0.0),
            recoil_dyn: SecondOrder::new(5.6, 0.60, 2.35, 0.0),
            sprint_dyn: SecondOrder::new(3.4, 0.90, 0.75, 0.0),
            action: ActionAnim::None,
            was_holding: false,
            was_slung: false,
            known_rounds: TUBE_CAPACITY,
            hammer_uncocked: false,
            walk_phase: 0.0,
            walk_speed: 0.0,
            idle_time: 0.0,
            shot_ads_factor: 1.0,
            shot_crouch_factor: 1.0,
            shot_roll_sign: 1.0,
            recoil_target_timer: None,
            cam_recoil_timer: Some(1.0),
            cam_pitch_kick: 0.0,
            cam_yaw_kick: 0.0,
            jolt_timer: 0.0,
            jolt_yaw_sign: 1.0,
            rng: 0x6D2B_79F5,
            skin_requested: false,
            skin: ResolvedSkin::default(),
        }
    }

    fn next_unit_f32(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        ((self.rng >> 8) as f32) * (1.0 / 16_777_216.0)
    }

    fn blocks_ads(&self) -> bool {
        matches!(
            self.action,
            ActionAnim::Draw { .. }
                | ActionAnim::ReloadIntro { .. }
                | ActionAnim::ReloadInsert { .. }
                | ActionAnim::ReloadEndNorm { .. }
                | ActionAnim::ReloadEndEmpty { .. }
                | ActionAnim::Inspect { .. }
        )
    }

    fn on_shot(&mut self, remaining_rounds: u32) {
        let aim = self.aim_dyn.y.clamp(0.0, 1.0);
        let sneaking = CLIENT_SNEAKING.load(Ordering::Relaxed);
        let ads_factor = 1.0 - 0.32 * aim;
        let crouch_factor = if sneaking { 0.875 } else { 1.0 };
        let variance = 0.88 + 0.24 * self.next_unit_f32();
        let yaw_rand = self.next_unit_f32() * 2.0 - 1.0;

        self.known_rounds = remaining_rounds;
        self.shot_ads_factor = ads_factor;
        self.shot_crouch_factor = crouch_factor;
        self.shot_roll_sign = if yaw_rand >= 0.0 { 1.0 } else { -1.0 };
        self.recoil_target_timer = Some(0.0);

        self.cam_recoil_timer = Some(0.0);
        self.cam_pitch_kick = 5.2 * ads_factor * crouch_factor * variance;
        self.cam_yaw_kick = 0.68 * ads_factor * yaw_rand;

        self.jolt_timer = 0.065;
        self.jolt_yaw_sign = self.shot_roll_sign;

        let keep_uncocked = remaining_rounds == 0;
        if keep_uncocked {
            self.hammer_uncocked = true;
        }
        self.action = ActionAnim::FireCycle {
            elapsed: 0.0,
            keep_uncocked,
            lever_sound_played: false,
            cock_sound_played: false,
            casing_ejected: false,
        };
    }

    fn on_dry_click(&mut self, rounds: u32) {
        self.known_rounds = rounds;
        if rounds == 0 {
            self.hammer_uncocked = true;
        }
        self.action = ActionAnim::DryClick { elapsed: 0.0 };
    }
}

fn client_anim() -> &'static Mutex<ClientAnimState> {
    CLIENT_ANIM.get_or_init(|| Mutex::new(ClientAnimState::new()))
}

fn aim_progress() -> f32 {
    f32::from_bits(ADS_BITS.load(Ordering::Relaxed)).clamp(0.0, 1.2)
}

fn holding_rifle_recently() -> bool {
    let seen = RIFLE_SEEN_FRAME.load(Ordering::Relaxed);
    let now = FRAME_SEQ.load(Ordering::Relaxed);
    seen > 0 && now.saturating_sub(seen) <= 2
}

// ---------------------------------------------------------------------------
// Pure-Rust Skin Decoder (`skins.rs` UTF-16LE SHA-1 + Base64 + Mojang JSON)
// ---------------------------------------------------------------------------

fn default_skin_for_uuid(uuid: u128) -> (&'static str, bool) {
    let most = (uuid >> 64) as u64;
    let least = uuid as u64;
    let xor = most ^ least;
    let folded = ((xor >> 32) as i32) ^ (xor as i32);
    let idx = folded.rem_euclid(18) as usize;
    DEFAULT_SKINS[idx]
}

fn sha1_hex_utf16le(text: &str) -> String {
    let mut msg = Vec::with_capacity(text.len() * 2 + 72);
    for unit in text.encode_utf16() {
        msg.extend_from_slice(&unit.to_le_bytes());
    }
    let bit_len = (msg.len() as u64) * 8;
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0x00);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    let mut h0: u32 = 0x6745_2301;
    let mut h1: u32 = 0xEFCD_AB89;
    let mut h2: u32 = 0x98BA_DCFE;
    let mut h3: u32 = 0x1032_5476;
    let mut h4: u32 = 0xC3D2_E1F0;

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let (mut a, mut b, mut c, mut d, mut e) = (h0, h1, h2, h3, h4);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(40);
    for word in [h0, h1, h2, h3, h4] {
        for byte in word.to_be_bytes() {
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0F) as usize] as char);
        }
    }
    out
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u8;
    for b in input.bytes() {
        if b.is_ascii_whitespace() {
            continue;
        }
        if b == b'=' {
            break;
        }
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        };
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

/// Extracts `("skins/<sha1>", is_slim)` from Mojang's Base64 `"textures"` property JSON.
fn parse_mojang_skin_property(b64_value: &str) -> Option<(String, bool)> {
    let raw = base64_decode(b64_value)?;
    let json = core::str::from_utf8(&raw).ok()?;
    let skin_idx = json.find("\"SKIN\"")?;
    let skin_tail = &json[skin_idx..];
    let skin_section = match skin_tail.find("\"CAPE\"") {
        Some(cape_pos) => &skin_tail[..cape_pos],
        None => skin_tail,
    };

    let url_key = skin_section.find("\"url\"")?;
    let after_colon = &skin_section[url_key + 5..];
    let first_quote = after_colon.find('"')?;
    let rest_url = &after_colon[first_quote + 1..];
    let end_quote = rest_url.find('"')?;
    let url = &rest_url[..end_quote];
    let hash = url.rsplit('/').next()?.trim();
    if hash.is_empty() || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }

    let slim = if let Some(model_pos) = skin_section.find("\"model\"") {
        let model_tail = &skin_section[model_pos + 7..];
        model_tail
            .split('"')
            .nth(1)
            .is_some_and(|m| m.eq_ignore_ascii_case("slim"))
    } else {
        false
    };

    let sprite = format!("skins/{}", sha1_hex_utf16le(hash));
    Some((sprite, slim))
}

// ---------------------------------------------------------------------------
// Player Arm Geometry (`wide` 4x12x4 + `slim` 3x12x4 + `0.25` 3D Sleeves)
// ---------------------------------------------------------------------------

struct LimbModel {
    limb: ModelPart,
    detached_sleeve: Option<ModelPart>,
}

struct ArmPair {
    right: LimbModel,
    left: LimbModel,
}

struct PlayerArms {
    wide: ArmPair,
    slim: ArmPair,
}

impl LimbModel {
    fn extract(root: &ModelPart, limb_name: &str, sleeve_name: &str) -> Option<Self> {
        let mut limb = root.get_child(limb_name)?.clone();
        limb.load_pose(PartPose::ZERO);
        let detached_sleeve = if limb.get_child(sleeve_name).is_some() {
            None
        } else {
            root.get_child(sleeve_name).cloned().map(|mut s| {
                s.load_pose(PartPose::ZERO);
                s
            })
        };
        Some(Self {
            limb,
            detached_sleeve,
        })
    }

    fn draw_part_with_skin(
        draw: &mut dyn BlockEntityDrawContext,
        part: &ModelPart,
        skin: &ResolvedSkin,
    ) {
        if !draw.model_part(part, &skin.primary, 0xFF_FFFF)
            && !draw.model_part(part, &skin.fallback, 0xFF_FFFF)
        {
            let _ = draw.model_part(part, DEFAULT_STEVE_WIDE, 0xFF_FFFF);
        }
    }

    fn draw(&self, draw: &mut dyn BlockEntityDrawContext, skin: &ResolvedSkin) {
        Self::draw_part_with_skin(draw, &self.limb, skin);
        if let Some(sleeve) = &self.detached_sleeve {
            Self::draw_part_with_skin(draw, sleeve, skin);
        }
    }
}

impl ArmPair {
    fn from_baked_root(root: &ModelPart) -> Option<Self> {
        Some(Self {
            right: LimbModel::extract(root, "right_arm", "right_sleeve")?,
            left: LimbModel::extract(root, "left_arm", "left_sleeve")?,
        })
    }

    fn procedural(slim: bool) -> Self {
        let width = if slim { 3.0 } else { 4.0 };
        let right_origin_x = if slim { -2.0 } else { -3.0 };
        let build_limb = |origin_x: f32, arm_uv: (i32, i32), sleeve_uv: (i32, i32)| -> LimbModel {
            let mut mesh = MeshDefinition::new();
            let root = mesh.root();
            let arm = mesh.add_or_replace_child(
                root,
                "arm",
                CubeListBuilder::create()
                    .tex_offs(arm_uv.0, arm_uv.1)
                    .add_box([origin_x, -2.0, -2.0], [width, 12.0, 4.0]),
                PartPose::ZERO,
            );
            mesh.add_or_replace_child(
                arm,
                "sleeve",
                CubeListBuilder::create()
                    .tex_offs(sleeve_uv.0, sleeve_uv.1)
                    .add_box_with(
                        [origin_x, -2.0, -2.0],
                        [width, 12.0, 4.0],
                        CubeDeformation::new(0.25),
                    ),
                PartPose::ZERO,
            );
            let baked = LayerDefinition::create(mesh, 64, 64).bake_root();
            let mut limb = baked.get_child("arm").cloned().unwrap_or(baked);
            limb.load_pose(PartPose::ZERO);
            LimbModel {
                limb,
                detached_sleeve: None,
            }
        };
        Self {
            right: build_limb(right_origin_x, (40, 16), (40, 32)),
            left: build_limb(-1.0, (32, 48), (48, 48)),
        }
    }
}

impl PlayerArms {
    fn bake(host: &dyn ClientHost) -> Self {
        let wide = host
            .bake_layer("minecraft:player#main")
            .and_then(|r| ArmPair::from_baked_root(&r))
            .unwrap_or_else(|| ArmPair::procedural(false));
        let slim = host
            .bake_layer("minecraft:player_slim#main")
            .and_then(|r| ArmPair::from_baked_root(&r))
            .unwrap_or_else(|| ArmPair::procedural(true));
        Self { wide, slim }
    }

    fn procedural_both() -> Self {
        Self {
            wide: ArmPair::procedural(false),
            slim: ArmPair::procedural(true),
        }
    }
}

// ---------------------------------------------------------------------------
// Standalone Model Resolution & Client Sound Helper
// ---------------------------------------------------------------------------

fn resolve_rifle_models(host: &dyn ClientHost) -> Option<RifleModels> {
    let assets = host.render_assets();
    Some(RifleModels {
        body: assets.model(&Identifier::parse(MODEL_BODY_ID).ok()?).ok()?,
        lever: assets.model(&Identifier::parse(MODEL_LEVER_ID).ok()?).ok()?,
        hammer: assets.model(&Identifier::parse(MODEL_HAMMER_ID).ok()?).ok()?,
        trigger: assets.model(&Identifier::parse(MODEL_TRIGGER_ID).ok()?).ok()?,
        gate: assets.model(&Identifier::parse(MODEL_GATE_ID).ok()?).ok()?,
        cartridge: assets
            .model(&Identifier::parse(MODEL_CARTRIDGE_ID).ok()?)
            .ok()?,
        casing: assets.model(&Identifier::parse(MODEL_CASING_ID).ok()?).ok()?,
        bullet_hole: assets
            .model(&Identifier::parse(MODEL_BULLET_HOLE_ID).ok()?)
            .ok()?,
    })
}

fn refresh_rifle_models(host: &dyn ClientHost) {
    *RIFLE_MODELS.lock().unwrap() = resolve_rifle_models(host);
}

fn play_local_foley(host: &dyn ClientHost, event_id: &str, volume: f32, pitch: f32) {
    let mut sound = SoundPlay::ui(event_id).with_volume(volume).with_pitch(pitch);
    sound.source = SoundSource::Players;
    let _ = host.play_local_sound(&sound, 0);
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
        let block_particle = r.particle_key(BLOCK_PARTICLE);
        r.item_part(winchester, Winchester);
        let _ = RIFLE_KEY.set(winchester);
        let _ = SMOKE_KEY.set(smoke);
        let _ = WHITE_SMOKE_KEY.set(white_smoke);
        let _ = FLAME_KEY.set(flame);
        let _ = POOF_KEY.set(poof);
        let _ = BLOCK_PARTICLE_KEY.set(block_particle);

        r.register_key_mapping(
            KeyRegistration::new(RELOAD_MAPPING, "key.keyboard.r")
                .label("key.blackpowder_legacy.reload")
                .context(KeyConflictContext::InGame),
        );
        r.register_key_mapping(
            KeyRegistration::new(INSPECT_MAPPING, "key.keyboard.i")
                .label("key.blackpowder_legacy.inspect")
                .context(KeyConflictContext::InGame),
        );

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
        if event.kind == BULLET && event.entity.is_none() {
            handle_bullet_terrain_impact(host, event);
        }
        Flow::Continue
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
    /// vanilla mining and melee punches).
    fn key_mapping_pre(
        &self,
        host: &mut dyn ClientHost,
        event: &mut KeyMappingPreEvent<'_>,
    ) -> Flow {
        let holding = holding_rifle_recently() || AIMING.load(Ordering::Relaxed);
        if event.mapping == ATTACK_MAPPING && holding && host.playing() {
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
        Flow::Continue
    }

    fn client_tick_pre(&self, host: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
        let tick = CLIENT_TICKS.fetch_add(1, Ordering::Relaxed) + 1;
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
        let mut want_inspect = false;
        while host.consume_click(MOD_ID, INSPECT_MAPPING) {
            want_inspect = true;
        }

        if !holding {
            PENDING_FIRE.store(false, Ordering::Relaxed);
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

        if AIMING.load(Ordering::Relaxed) {
            let mut anim = client_anim().lock().unwrap();
            if matches!(anim.action, ActionAnim::Inspect { .. }) {
                anim.action = ActionAnim::None;
            }
        }

        if CLIENT_RELOADING.load(Ordering::Relaxed) && AIMING.swap(false, Ordering::Relaxed) {
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

        // Step world decals & tumbling/bouncing .44-40 brass casings even when the player switches
        // away from the rifle so airborne shells finish landing and clinking naturally.
        step_world_effects(host, dt);

        let holding = holding_rifle_recently() || AIMING.load(Ordering::Relaxed);
        let sprinting = CLIENT_SPRINTING.load(Ordering::Relaxed);

        let mut cam_delta_pitch = 0.0_f32;
        let mut cam_delta_yaw = 0.0_f32;
        let mut triggered_foley: Vec<(&'static str, f32, f32)> = Vec::new();
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
                            triggered_foley.push((SND_LEVER, 0.86, 1.0));
                        }
                        if !casing_ejected && next >= CASING_HANDOFF_SECS {
                            casing_ejected = true;
                            eject_casing_aim = Some(aim_y.clamp(0.0, 1.0));
                        }
                        if !cock_sound_played && next >= LEVER_DELAY_SECS + 0.24 {
                            cock_sound_played = true;
                            anim.hammer_uncocked = false;
                            triggered_foley.push((SND_COCK, 0.72, 1.04));
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
                        triggered_foley.push((SND_LEVER, 0.90, 0.98));
                    }
                    if !cock_sound_played && next >= 0.40 {
                        cock_sound_played = true;
                        anim.hammer_uncocked = false;
                        triggered_foley.push((SND_COCK, 0.78, 1.0));
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

        for (snd, vol, pitch) in triggered_foley {
            play_local_foley(host, snd, vol, pitch);
        }

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
            let scale = 1.0 - (1.0 - AIM_FOV) * aim;
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
            let sens_scale = 1.0 - (1.0 - AIM_FOV) * aim * 0.85;
            event.mouse_sensitivity *= f64::from(sens_scale);
        }
    }

    fn camera_pose(&self, _: &dyn ClientHost, event: &mut CameraPoseEvent<'_>) {
        let (jolt, sign) = {
            let anim = client_anim().lock().unwrap();
            (anim.jolt_timer, anim.jolt_yaw_sign)
        };
        if jolt > 0.0 {
            let u = (jolt / 0.065).clamp(0.0, 1.0);
            let wave = (PI * (1.0 - u) * 2.0).sin() * u;
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
            if let Some((off_rounds, chest_armored)) = local_offhand_rifle_status(host) {
                let left = cached.is_some_and(|s| s.main_arm_left);
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
            //    (`RIFLE_DISPLAY_TRANS`, `0.45` scale) + `[-0.5; 3]` unit-block centering.
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
        if (event.layer == "minecraft:crosshair" || event.layer == "crosshair")
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
        let aim = aim_progress().clamp(0.0, 1.0);
        let [wf, hf] = event.batch.size();
        let (w, h) = (wf as i32, hf as i32);

        if aim >= 0.02 {
            let brightness =
                ((1.0 - (1.0 - AIM_VIGNETTE) * aim) * 255.0).clamp(0.0, 255.0) as u32;
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

        // Compact western brass ammo counter to the right of the hotbar (`8 / 8`).
        let (rounds_now, reloading) = {
            let anim = client_anim().lock().unwrap();
            (
                anim.known_rounds,
                CLIENT_RELOADING.load(Ordering::Relaxed),
            )
        };
        let box_w = 52.0_f32;
        let box_h = 15.0_f32;
        let x0 = (wf * 0.5 + 98.0).min(wf - box_w - 6.0);
        let y0 = hf - 19.0;
        let border = if reloading {
            0xCC_E3_B3_4C
        } else if rounds_now == 0 {
            0xCC_C9_4A_38
        } else {
            0xAA_8C_6D_3F
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

// ---------------------------------------------------------------------------
// TAC-Style Keyframe Weapon & Dual-Arm Pose Evaluator
// ---------------------------------------------------------------------------

struct WeaponPose {
    gun_trans: [f32; 3],
    gun_rot: [f32; 3],
    lever_deg: f32,
    hammer_deg: f32,
    trigger_deg: f32,
    gate_deg: f32,
    show_cartridge: bool,
    cartridge_tex: [f32; 3],
    cartridge_rot: [f32; 3],
    show_casing: bool,
    casing_tex: [f32; 3],
    casing_rot: [f32; 3],
    right_hand_tex: [f32; 3],
    right_hand_rot: [f32; 3],
    left_hand_tex: [f32; 3],
    left_hand_rot: [f32; 3],
}

#[allow(clippy::too_many_arguments)]
fn evaluate_weapon_pose(
    aim: f32,
    equip: f32,
    kick: f32,
    sprint_blend: f32,
    ads_factor: f32,
    crouch_factor: f32,
    roll_sign: f32,
    walk_phase: f32,
    walk_speed: f32,
    idle_time: f32,
    hammer_uncocked: bool,
    action: ActionAnim,
) -> WeaponPose {
    let aim_clamped = aim.clamp(0.0, 1.0);
    let hip = 1.0 - aim_clamped;

    // Shoulder-pocket parabolic arc dip while raising/lowering iron sights.
    let arc_dip = -0.030 * (4.0 * aim_clamped * hip);
    let mut gun_trans = [
        ADS_OFFSET[0] * aim,
        ADS_OFFSET[1] * aim + arc_dip,
        ADS_OFFSET[2] * aim,
    ];
    let mut gun_rot = [0.0, 0.0, 3.2 * hip];

    // Subtle idle breathing + walk Lissajous bobbing (damped 88% in ADS).
    let sway_scale = 0.12 + 0.88 * hip;
    gun_trans[0] += walk_phase.cos() * 0.013 * walk_speed * sway_scale;
    gun_trans[1] += (idle_time * 1.75).sin() * 0.0025 * sway_scale
        - (walk_phase * 2.0).sin().abs() * 0.011 * walk_speed * sway_scale;
    gun_rot[0] += (idle_time * 1.75 + 0.6).cos() * 0.32 * sway_scale;
    gun_rot[2] += walk_phase.sin() * 1.35 * walk_speed * sway_scale;

    // Smooth sprint low-ready carry across chest.
    if sprint_blend > 1e-3 {
        gun_trans[0] -= 0.105 * sprint_blend;
        gun_trans[1] -= 0.072 * sprint_blend;
        gun_trans[2] += 0.028 * sprint_blend;
        gun_rot[0] -= 11.5 * sprint_blend;
        gun_rot[1] += 25.0 * sprint_blend;
        gun_rot[2] += 13.5 * sprint_blend;
    }

    // Rear-stock-pivoted 2nd-order visual recoil kick.
    if kick.abs() > 1e-4 {
        let pitch_deg = 7.2 * kick * ads_factor * crouch_factor;
        let pitch_rad = pitch_deg * (PI / 180.0);
        let pivot = model_texel_to_hand(RECOIL_PIVOT_TEXELS);
        let py = HAND_BASE[1] - equip * 0.6 + pivot[1];
        let pz = HAND_BASE[2] + pivot[2];
        let dy = py - (py * pitch_rad.cos() - pz * pitch_rad.sin());
        let dz = pz - (py * pitch_rad.sin() + pz * pitch_rad.cos());

        gun_trans[1] += dy + 0.008 * kick * ads_factor;
        gun_trans[2] += dz + 0.095 * kick * ads_factor;
        gun_rot[0] += pitch_deg;
        gun_rot[1] += 0.85 * kick * ads_factor * roll_sign;
        gun_rot[2] += 1.30 * kick * ads_factor * roll_sign;
    }

    let mut lever_deg = 0.0_f32;
    let mut hammer_deg = if hammer_uncocked { -36.0 } else { 0.0 };
    let mut trigger_deg = 0.0_f32;
    let mut gate_deg = 0.0_f32;
    let mut show_cartridge = false;
    let mut cartridge_tex = [11.2, 8.2, 11.2];
    let mut cartridge_rot = [0.0, 0.0, 0.0];
    let mut show_casing = false;
    let mut casing_tex = EJECTOR_START_TEXELS;
    let mut casing_rot = [0.0, 0.0, 0.0];

    let base_right_tex = [
        GRIP_WRIST_TEXELS[0] - 0.20 * aim_clamped,
        GRIP_WRIST_TEXELS[1] - 0.35 * aim_clamped,
        GRIP_WRIST_TEXELS[2] + 0.10 * aim_clamped,
    ];
    let base_right_rot = [
        124.0 + 12.0 * aim_clamped,
        18.0 - 6.0 * aim_clamped,
        -30.0 + 4.0 * aim_clamped,
    ];
    let base_left_tex = [
        FORESTOCK_TEXELS[0],
        FORESTOCK_TEXELS[1] - 0.25 * aim_clamped,
        FORESTOCK_TEXELS[2],
    ];
    let base_left_rot = [
        118.0 + 4.0 * aim_clamped,
        -16.0 + 3.0 * aim_clamped,
        14.0 - 2.0 * aim_clamped,
    ];

    let mut right_hand_tex = base_right_tex;
    let mut right_hand_rot = base_right_rot;
    let mut left_hand_tex = base_left_tex;
    let mut left_hand_rot = base_left_rot;

    match action {
        ActionAnim::None => {}
        ActionAnim::Draw { elapsed } => {
            let p = smoothstep((elapsed / DRAW_DURATION_SECS).clamp(0.0, 1.0));
            let inv = 1.0 - p;
            gun_trans[0] += 0.075 * inv;
            gun_trans[1] -= 0.27 * inv;
            gun_trans[2] += 0.055 * inv;
            gun_rot[0] -= 24.0 * inv;
            gun_rot[1] -= 13.0 * inv;
            gun_rot[2] += 17.0 * inv;
        }
        ActionAnim::FireCycle {
            elapsed,
            keep_uncocked,
            ..
        } => {
            // Trigger pull + hammer fall (`0° -> -36°` in `50 ms`).
            let strike = (elapsed / 0.05).clamp(0.0, 1.0);
            trigger_deg = -20.0 * (1.0 - ((elapsed - 0.06).max(0.0) / 0.14).clamp(0.0, 1.0));
            hammer_deg = -36.0 * strike;

            if keep_uncocked {
                if (CASING_LAST_RISE_START_SECS..CASING_LAST_HANDOFF_SECS).contains(&elapsed) {
                    let u = ((elapsed - CASING_LAST_RISE_START_SECS)
                        / (CASING_LAST_HANDOFF_SECS - CASING_LAST_RISE_START_SECS))
                        .clamp(0.0, 1.0);
                    show_casing = true;
                    casing_tex = lerp3(EJECTOR_START_TEXELS, EJECTOR_HANDOFF_TEXELS, u);
                    casing_rot = [18.0 * u, -8.0 * u, 24.0 * u];
                }
            } else if elapsed >= LEVER_DELAY_SECS {
                if (CASING_RISE_START_SECS..CASING_HANDOFF_SECS).contains(&elapsed) {
                    let u = ((elapsed - CASING_RISE_START_SECS)
                        / (CASING_HANDOFF_SECS - CASING_RISE_START_SECS))
                        .clamp(0.0, 1.0);
                    show_casing = true;
                    casing_tex = lerp3(EJECTOR_START_TEXELS, EJECTOR_HANDOFF_TEXELS, u);
                    casing_rot = [18.0 * u, -8.0 * u, 24.0 * u];
                }
                let lt = ((elapsed - LEVER_DELAY_SECS) / LEVER_DURATION_SECS).clamp(0.0, 1.0);
                let lever_curve = if lt < 0.45 {
                    smoothstep(lt / 0.45)
                } else {
                    1.0 - smoothstep((lt - 0.45) / 0.55)
                };
                lever_deg = 48.0 * lever_curve;
                hammer_deg = if lt < 0.48 {
                    lerp(-36.0, 7.5, smoothstep(lt / 0.48))
                } else {
                    lerp(7.5, 0.0, smoothstep((lt - 0.48) / 0.52))
                };

                let damp = 1.0 - 0.40 * aim_clamped;
                gun_trans[1] -= 0.028 * lever_curve * damp;
                gun_trans[2] += 0.015 * lever_curve * damp;
                gun_rot[0] -= 2.4 * lever_curve * damp;
                gun_rot[2] += 7.5 * lever_curve * damp;

                right_hand_tex = [
                    base_right_tex[0],
                    base_right_tex[1] - 2.15 * lever_curve,
                    base_right_tex[2] - 1.95 * lever_curve,
                ];
                right_hand_rot[0] = base_right_rot[0] + 16.0 * lever_curve;
            }
        }
        ActionAnim::DryClick { elapsed } => {
            let p = 1.0 - (elapsed / 0.22).clamp(0.0, 1.0);
            trigger_deg = -20.0 * p;
            gun_trans[2] += 0.009 * p;
            gun_rot[0] -= 1.1 * p;
        }
        ActionAnim::ReloadIntro { elapsed } => {
            let open_secs = (RELOAD_OPEN_TICKS as f32) * 0.05;
            let p = smoothstep((elapsed / open_secs).clamp(0.0, 1.0));
            apply_reload_cradle(&mut gun_trans, &mut gun_rot, p);
            left_hand_tex = lerp3(base_left_tex, [12.10, 7.20, 12.20], p);
            left_hand_rot = lerp3(base_left_rot, [122.0, -12.0, 24.0], p);
        }
        ActionAnim::ReloadInsert { elapsed } => {
            apply_reload_cradle(&mut gun_trans, &mut gun_rot, 1.0);
            let shell_secs = (RELOAD_INSERT_TICKS as f32) * 0.05;
            let u = (elapsed / shell_secs).clamp(0.0, 1.0);

            if u < 0.76 {
                show_cartridge = true;
                if u < 0.32 {
                    let k = smoothstep(u / 0.32);
                    cartridge_tex = lerp3([11.80, 6.60, 12.60], [9.65, 10.55, 11.20], k);
                    cartridge_rot = lerp3([22.0, -34.0, 12.0], [4.0, -18.0, 0.0], k);
                } else {
                    let k = smoothstep((u - 0.32) / 0.44);
                    cartridge_tex = lerp3([9.65, 10.55, 11.20], [8.35, 10.58, 9.25], k);
                    cartridge_rot = lerp3([4.0, -18.0, 0.0], [0.0, -4.0, 0.0], k);
                }
            }

            gate_deg = if (0.24..0.78).contains(&u) {
                let g = if u < 0.48 {
                    smoothstep((u - 0.24) / 0.24)
                } else {
                    1.0 - smoothstep((u - 0.48) / 0.30)
                };
                -23.0 * g
            } else {
                0.0
            };

            let seat_bump = (-((u - 0.62) * 7.5).powi(2)).exp();
            gun_trans[0] -= 0.010 * seat_bump;
            gun_trans[1] += 0.008 * seat_bump;
            gun_rot[2] -= 2.4 * seat_bump;

            left_hand_tex = if u < 0.32 {
                lerp3([12.20, 7.10, 12.35], [11.35, 8.05, 11.75], smoothstep(u / 0.32))
            } else if u < 0.70 {
                lerp3(
                    [11.35, 8.05, 11.75],
                    [10.85, 8.35, 11.35],
                    smoothstep((u - 0.32) / 0.38),
                )
            } else {
                lerp3(
                    [10.85, 8.35, 11.35],
                    [12.20, 7.10, 12.35],
                    smoothstep((u - 0.70) / 0.30),
                )
            };
            left_hand_rot = [122.0, -12.0 + 4.0 * seat_bump, 24.0];
        }
        ActionAnim::ReloadEndNorm { elapsed } => {
            let dur = (RELOAD_CLOSE_NORM_TICKS as f32) * 0.05;
            let p = 1.0 - smoothstep((elapsed / dur).clamp(0.0, 1.0));
            apply_reload_cradle(&mut gun_trans, &mut gun_rot, p);
            left_hand_tex = lerp3(base_left_tex, [12.10, 7.20, 12.20], p);
            left_hand_rot = lerp3(base_left_rot, [122.0, -12.0, 24.0], p);
        }
        ActionAnim::ReloadEndEmpty { elapsed, .. } => {
            let dur = (RELOAD_CLOSE_EMPTY_TICKS as f32) * 0.05;
            let settle = 1.0 - smoothstep((elapsed / 0.28).clamp(0.0, 1.0));
            apply_reload_cradle(&mut gun_trans, &mut gun_rot, settle);
            left_hand_tex = lerp3(base_left_tex, [12.10, 7.20, 12.20], settle);
            left_hand_rot = lerp3(base_left_rot, [122.0, -12.0, 24.0], settle);

            if elapsed >= 0.15 {
                let lt = ((elapsed - 0.15) / (dur - 0.17)).clamp(0.0, 1.0);
                let lever_curve = if lt < 0.46 {
                    smoothstep(lt / 0.46)
                } else {
                    1.0 - smoothstep((lt - 0.46) / 0.54)
                };
                lever_deg = 50.0 * lever_curve;
                hammer_deg = if lt < 0.48 {
                    lerp(-36.0, 8.0, smoothstep(lt / 0.48))
                } else {
                    lerp(8.0, 0.0, smoothstep((lt - 0.48) / 0.52))
                };
                gun_rot[2] += 8.0 * lever_curve;
                gun_rot[0] -= 3.0 * lever_curve;
                gun_trans[2] += 0.024 * lever_curve;
                right_hand_tex = [
                    base_right_tex[0],
                    base_right_tex[1] - 2.25 * lever_curve,
                    base_right_tex[2] - 2.05 * lever_curve,
                ];
                right_hand_rot[0] = base_right_rot[0] + 17.0 * lever_curve;
            }
        }
        ActionAnim::Inspect { elapsed } => {
            let u = (elapsed / INSPECT_DURATION_SECS).clamp(0.0, 1.0);
            let (tx, ty, tz, rx, ry, rz, l_shift) = if u < 0.16 {
                let k = smoothstep(u / 0.16);
                (
                    -0.14 * k,
                    0.05 * k,
                    0.06 * k,
                    8.0 * k,
                    18.0 * k,
                    -28.0 * k,
                    k,
                )
            } else if u < 0.42 {
                let drift = ((u - 0.16) / 0.26 * PI).sin();
                (
                    -0.14,
                    0.05 + 0.012 * drift,
                    0.06,
                    8.0 + 3.0 * drift,
                    18.0 + 5.0 * drift,
                    -28.0 - 4.0 * drift,
                    1.0,
                )
            } else if u < 0.58 {
                let k = smoothstep((u - 0.42) / 0.16);
                (
                    lerp(-0.14, -0.18, k),
                    lerp(0.05, 0.03, k),
                    lerp(0.06, 0.04, k),
                    lerp(8.0, 12.0, k),
                    lerp(18.0, -24.0, k),
                    lerp(-28.0, 26.0, k),
                    1.0 - 0.5 * k,
                )
            } else if u < 0.82 {
                let drift = ((u - 0.58) / 0.24 * PI).sin();
                (
                    -0.18,
                    0.03 + 0.010 * drift,
                    0.04,
                    12.0 - 2.5 * drift,
                    -24.0 - 4.0 * drift,
                    26.0 + 4.0 * drift,
                    0.5,
                )
            } else {
                let k = 1.0 - smoothstep((u - 0.82) / 0.18);
                (
                    -0.18 * k,
                    0.03 * k,
                    0.04 * k,
                    12.0 * k,
                    -24.0 * k,
                    26.0 * k,
                    0.5 * k,
                )
            };

            gun_trans[0] += tx;
            gun_trans[1] += ty;
            gun_trans[2] += tz;
            gun_rot[0] += rx;
            gun_rot[1] += ry;
            gun_rot[2] += rz;
            left_hand_tex = lerp3(base_left_tex, [8.55, 8.15, 5.10], l_shift);
            left_hand_rot = lerp3(base_left_rot, [120.0, -14.0, 16.0], l_shift);
        }
    }

    WeaponPose {
        gun_trans,
        gun_rot,
        lever_deg,
        hammer_deg,
        trigger_deg,
        gate_deg,
        show_cartridge,
        cartridge_tex,
        cartridge_rot,
        show_casing,
        casing_tex,
        casing_rot,
        right_hand_tex,
        right_hand_rot,
        left_hand_tex,
        left_hand_rot,
    }
}

fn apply_reload_cradle(gun_trans: &mut [f32; 3], gun_rot: &mut [f32; 3], weight: f32) {
    gun_trans[0] -= 0.072 * weight;
    gun_trans[1] -= 0.044 * weight;
    gun_trans[2] += 0.044 * weight;
    gun_rot[0] += 6.8 * weight;
    gun_rot[1] += 11.5 * weight;
    gun_rot[2] -= 20.5 * weight;
}

fn draw_gun_space_arm(
    draw: &mut dyn BlockEntityDrawContext,
    limb: &LimbModel,
    skin: &ResolvedSkin,
    is_right_arm: bool,
    fist_tex: [f32; 3],
    rot_deg: [f32; 3],
) {
    let shaft_center_x = match (is_right_arm, skin.slim) {
        (true, false) => -1.0 / 16.0,
        (true, true) => -0.5 / 16.0,
        (false, false) => 1.0 / 16.0,
        (false, true) => 0.5 / 16.0,
    };
    let sx = ARM_GUN_SPACE_WIDTH;
    let sy = ARM_GUN_SPACE_LENGTH;

    draw.push();
    draw.translate(tex_to_unit(fist_tex));
    draw.rotate_about([0.0, 1.0, 0.0], rot_deg[1], [0.0; 3]);
    draw.rotate_about([1.0, 0.0, 0.0], rot_deg[0], [0.0; 3]);
    draw.rotate_about([0.0, 0.0, 1.0], rot_deg[2], [0.0; 3]);
    draw.translate([-sx * shaft_center_x, sy * (9.5 / 16.0), 0.0]);
    draw.scale([sx, -sy, -sx]);
    limb.draw(draw, skin);
    draw.pop();
}

/// Draws the 5-part articulated Winchester 1873 (`body` + `lever` + `hammer` + `trigger` + `gate`)
/// inside `[0..1]` unit-block rifle model space (`1` unit = `16` JSON texels). Shared by both
/// 1st-person `render_hand` and 3rd-person diagonal back-sling `render_player_post`.
fn draw_rifle_assembly(
    draw: &mut dyn BlockEntityDrawContext,
    models: RifleModels,
    lever_deg: f32,
    hammer_deg: f32,
    trigger_deg: f32,
    gate_deg: f32,
) -> bool {
    if draw.model(models.body, 0xFF_FF_FF_FF).is_err() {
        return false;
    }

    draw.push();
    draw.rotate_about([1.0, 0.0, 0.0], lever_deg, tex_to_unit(LEVER_PIVOT_TEX));
    let _ = draw.model(models.lever, 0xFF_FF_FF_FF);
    draw.pop();

    draw.push();
    draw.rotate_about([1.0, 0.0, 0.0], hammer_deg, tex_to_unit(HAMMER_PIVOT_TEX));
    let _ = draw.model(models.hammer, 0xFF_FF_FF_FF);
    draw.pop();

    draw.push();
    draw.rotate_about([1.0, 0.0, 0.0], trigger_deg, tex_to_unit(TRIGGER_PIVOT_TEX));
    let _ = draw.model(models.trigger, 0xFF_FF_FF_FF);
    draw.pop();

    draw.push();
    draw.rotate_about([0.0, 1.0, 0.0], gate_deg, tex_to_unit(GATE_PIVOT_TEX));
    let _ = draw.model(models.gate, 0xFF_FF_FF_FF);
    draw.pop();

    true
}

/// Returns `(loaded_rounds, chest_armored)` when the local player has the Winchester 1873
/// stowed in their off-hand (`Hand::Off`).
fn local_offhand_rifle_status(host: &dyn ClientHost) -> Option<(u32, bool)> {
    let rifle = RIFLE_KEY.get().copied()?;
    let cam = host.camera_entity()?;
    let liv = host.living(cam)?;
    let off = liv.hand_item(host, Hand::Off)?;
    if off.item() != rifle {
        return None;
    }
    let chest_armored = liv.equipment(host, EquipmentSlot::Chest).is_some();
    Some((rounds(&off), chest_armored))
}

/// Converts `[0..16]` JSON texels into right-hand view space after `RIFLE_DISPLAY`.
fn model_texel_to_hand(tex: [f32; 3]) -> [f32; 3] {
    let k = RIFLE_DISPLAY_SCALE / 16.0;
    [
        RIFLE_DISPLAY_TRANS[0] + (tex[0] - 8.0) * k,
        RIFLE_DISPLAY_TRANS[1] + (tex[1] - 8.0) * k,
        RIFLE_DISPLAY_TRANS[2] + (tex[2] - 8.0) * k,
    ]
}

/// Evaluates any `[0..16]` rifle JSON texel point (`MUZZLE_TEXELS`, `EJECTOR_HANDOFF_TEXELS`, …)
/// through the exact `render_hand` transform stack (`RIFLE_DISPLAY` -> `HAND_BASE` -> `gun_rot`
/// -> `gun_trans`) in 1st-person hand camera space.
fn compute_gun_texel_view_pos(tex: [f32; 3], pose: &WeaponPose, equip: f32) -> [f32; 3] {
    let local = model_texel_to_hand(tex);
    let mut x = HAND_BASE[0] + local[0];
    let mut y = HAND_BASE[1] - equip * 0.6 + local[1];
    let mut z = HAND_BASE[2] + local[2];

    let (sz, cz) = pose.gun_rot[2].to_radians().sin_cos();
    (x, y) = (x * cz - y * sz, x * sz + y * cz);
    let (sy, cy) = pose.gun_rot[1].to_radians().sin_cos();
    (x, z) = (x * cy + z * sy, -x * sy + z * cy);
    let (sx, cx) = pose.gun_rot[0].to_radians().sin_cos();
    (y, z) = (y * cx - z * sx, y * sx + z * cx);

    [
        x + pose.gun_trans[0],
        y + pose.gun_trans[1],
        z + pose.gun_trans[2],
    ]
}

/// Builds `ACTION_FIRE` with the exact screen-ray-matched world muzzle offset (`mm`) so muzzle
/// smoke & flash appear right on `boca_cano` in hipfire, mid-ADS, full ADS, and `F5` 3rd person
/// across any Minecraft FOV setting.
fn build_fire_input() -> RifleInput {
    let aim_pct = (aim_progress().clamp(0.0, 1.0) * 100.0).round() as i32;
    let (right_m, up_m, fwd_m) = if holding_rifle_recently() {
        let mx = f32::from_bits(LAST_MUZZLE_VIEW_X.load(Ordering::Relaxed));
        let my = f32::from_bits(LAST_MUZZLE_VIEW_Y.load(Ordering::Relaxed));
        let mz = f32::from_bits(LAST_MUZZLE_VIEW_Z.load(Ordering::Relaxed));
        let fwd_1p = (-mz).clamp(0.5, 3.0);
        let world_fov = f32::from_bits(LAST_WORLD_FOV_BITS.load(Ordering::Relaxed)).clamp(20.0, 150.0);
        // Horizon's 1st-person hand pass uses a 70° vertical FOV (`tan(35°)`), while world
        // particles project under `world_fov`. Scaling `(mx, my)` by `fov_ratio * (1.35 / fwd_1p)`
        // keeps exact 0-pixel ray alignment with `boca_cano` on screen at `1.35` blocks barrel reach.
        let fov_ratio = (world_fov.to_radians() * 0.5).tan() / (35.0_f32.to_radians()).tan();
        let target_fwd = 1.35_f32;
        let ray_scale = target_fwd / fwd_1p;
        (
            (mx * fov_ratio * ray_scale).clamp(-1.1, 1.1),
            (my * fov_ratio * ray_scale).clamp(-1.1, 1.1),
            target_fwd,
        )
    } else {
        // 3rd-person (`F5`) held barrel tip offset relative to eye basis.
        (0.28_f32, -0.14_f32, 1.35_f32)
    };

    RifleInput {
        action: VarInt(ACTION_FIRE),
        muzzle_right_mm: VarInt((right_m * 1000.0).round() as i32),
        muzzle_up_mm: VarInt((up_m * 1000.0).round() as i32),
        muzzle_fwd_mm: VarInt((fwd_m * 1000.0).round() as i32),
        aim_pct: VarInt(aim_pct),
    }
}

const fn tex_to_unit(p: [f32; 3]) -> [f32; 3] {
    [p[0] / 16.0, p[1] / 16.0, p[2] / 16.0]
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        lerp(a[0], b[0], t),
        lerp(a[1], b[1], t),
        lerp(a[2], b[2], t),
    ]
}

fn smoothstep(x: f32) -> f32 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn cam_recoil_curve(t: f32) -> (f32, f32) {
    const CLIMB_END: f32 = 0.095;
    const HOLD_END: f32 = 0.185;
    const REC_END: f32 = 0.380;
    const RECOVERY_FRAC: f32 = 1.0 / 1.75;

    if t <= 0.0 {
        (0.0, 0.0)
    } else if t < CLIMB_END {
        let u = (t / CLIMB_END).clamp(0.0, 1.0);
        let c = 0.5 * (1.0 - (PI * u).cos());
        (c, c)
    } else if t < HOLD_END {
        (1.0, 1.0)
    } else if t < REC_END {
        let u = ((t - HOLD_END) / (REC_END - HOLD_END)).clamp(0.0, 1.0);
        let ease = 0.5 * (1.0 - (PI * u).cos());
        (1.0 - RECOVERY_FRAC * ease, 1.0 - 0.5 * ease)
    } else {
        (1.0 - RECOVERY_FRAC, 0.5)
    }
}

fn recoil_envelope(t: f32) -> f32 {
    if !(0.0..0.20).contains(&t) {
        return 0.0;
    }
    if t < 0.035 {
        t / 0.035
    } else {
        let u = (t - 0.035) / (0.20 - 0.035);
        (1.0 - u) * (1.0 - u)
    }
}

// ---------------------------------------------------------------------------
// Network Payload Handlers
// ---------------------------------------------------------------------------

fn on_rifle_input(value: RifleInput, cx: &mut dyn IPayloadContext) -> Result<(), String> {
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

fn on_rifle_sync(value: RifleSync, cx: &mut dyn IPayloadContext) -> Result<(), String> {
    let rounds_now = (value.rounds.0.max(0) as u32).min(TUBE_CAPACITY);
    let mut foley: Option<(&'static str, f32, f32)> = None;

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
                foley = Some((SND_COCK, 0.72, 1.32));
            }
            SYNC_RELOAD_START => {
                CLIENT_RELOADING.store(true, Ordering::Relaxed);
                AIMING.store(false, Ordering::Relaxed);
                anim.action = ActionAnim::ReloadIntro { elapsed: 0.0 };
                foley = Some((SND_RELOAD_INTRO, 0.90, 1.0));
            }
            SYNC_SHELL_INSERTED => {
                CLIENT_RELOADING.store(true, Ordering::Relaxed);
                anim.action = ActionAnim::ReloadInsert { elapsed: 0.0 };
                foley = Some((SND_RELOAD_INSERT, 0.92, 1.0));
            }
            SYNC_RELOAD_END_NORM => {
                CLIENT_RELOADING.store(false, Ordering::Relaxed);
                anim.action = ActionAnim::ReloadEndNorm { elapsed: 0.0 };
                foley = Some((SND_RELOAD_END, 0.88, 1.0));
            }
            SYNC_RELOAD_END_EMPTY => {
                CLIENT_RELOADING.store(false, Ordering::Relaxed);
                anim.action = ActionAnim::ReloadEndEmpty {
                    elapsed: 0.0,
                    lever_sound_played: false,
                    cock_sound_played: false,
                };
                foley = Some((SND_RELOAD_END, 0.85, 1.0));
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

    if let Some((snd, vol, pitch)) = foley
        && let Ok(host) = cx.client_host()
    {
        play_local_foley(host, snd, vol, pitch);
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

// ---------------------------------------------------------------------------
// Server-Authoritative Weapon Logic & ItemPart
// ---------------------------------------------------------------------------

struct Winchester;

fn rounds(stack: &ItemStack) -> u32 {
    stack.damage().min(TUBE_CAPACITY)
}

fn shooter(host: &dyn ServerHost) -> Option<mod_api::PlayerRef> {
    host.players().acting_player()
}

fn is_player_reloading(uuid: u128) -> bool {
    RELOAD_SESSIONS.lock().unwrap().contains_key(&uuid)
}

fn cancel_reload_session(host: &mut dyn ServerHost, player: mod_api::PlayerRef) -> bool {
    let removed = RELOAD_SESSIONS.lock().unwrap().remove(&player.uuid()).is_some();
    if removed {
        let cur = held_rifle(host, player).map_or(0, |(_, r)| rounds(&r));
        send_sync(host, player, SYNC_RELOAD_ABORTED, cur);
    }
    removed
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

fn muzzle_effects(
    host: &mut dyn ServerHost,
    player: mod_api::PlayerRef,
    smoke: ParticleKey,
    muzzle_mm: [i32; 3],
    aim_pct: i32,
) {
    let entity = player.entity();
    let Some(level) = entity.level(host) else {
        return;
    };
    let Some(eye) = entity.eye_position(host) else {
        return;
    };
    let Some(forward) = entity.view_vector(host) else {
        return;
    };
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
    let aimed = aim_pct >= 65;

    let muzzle = [
        eye[0] + right[0] * right_m + up[0] * up_m + forward[0] * fwd_m,
        eye[1] + right[1] * right_m + up[1] * up_m + forward[1] * fwd_m,
        eye[2] + right[2] * right_m + up[2] * up_m + forward[2] * fwd_m,
    ];

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
            return;
        }
    }

    let lever_ready = LEVER_READY.lock().unwrap().get(&uuid).copied().unwrap_or(0);
    if lever_ready > now {
        return;
    }

    if loaded == 0 {
        LEVER_READY.lock().unwrap().insert(uuid, now + 5);
        send_sync(host, player, SYNC_DRY_CLICK, 0);
        return;
    }

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
    let shot =
        host.spawn_projectile_from_rotation(owner, &bullet, &carried, 0.0, BULLET_SPEED, spread);
    if shot.is_none() {
        return;
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
    play_world_sound(host, player, SND_FIRE, 2.5, 1.0);
    play_world_sound(host, player, SND_THUMP, 1.15, 1.0);
    muzzle_effects(host, player, smoke, muzzle_mm, aim_pct);
    send_sync(host, player, SYNC_SHOT, remaining);
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
}

fn tick_reload_sessions(host: &mut dyn ServerHost, now: u64) {
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
        let valid_rifle = held_rifle(host, player).filter(|(sel, _)| *sel == session.slot);

        let Some((slot, rifle)) = valid_rifle else {
            RELOAD_SESSIONS.lock().unwrap().remove(&uuid);
            send_sync(host, player, SYNC_RELOAD_ABORTED, 0);
            continue;
        };
        let loaded = rounds(&rifle);
        if sprinting {
            RELOAD_SESSIONS.lock().unwrap().remove(&uuid);
            send_sync(host, player, SYNC_RELOAD_ABORTED, loaded);
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
                        let empty_chamber = session.from_empty && session.inserted_any && loaded > 0;
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
                    } else {
                        let next_loaded = loaded + 1;
                        let mut rifle = rifle;
                        rifle.set_damage(next_loaded);
                        if player.set_inventory_slot(host, slot, Some(rifle)).is_err() {
                            remove_session = true;
                            send_sync(host, player, SYNC_RELOAD_ABORTED, loaded);
                        } else {
                            session.inserted_any = true;
                            send_sync(host, player, SYNC_SHELL_INSERTED, next_loaded);
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
                    LEVER_READY.lock().unwrap().insert(
                        uuid,
                        now + if empty_chamber { 2 } else { 1 },
                    );
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
        *event.bar = Some((width, AMMO_BAR_RGB));
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

// ---------------------------------------------------------------------------
// World Bullet-Impact Decals & 3D Ejected .44-40 Brass Casings
// ---------------------------------------------------------------------------

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

fn handle_bullet_terrain_impact(host: &mut dyn ServerHost, event: &ProjectileImpactEvent<'_>) {
    let face = normalize_cardinal_face(event.face);
    let impact_fx_pos = [
        event.at[0] + f64::from(face[0]) * 0.055,
        event.at[1] + f64::from(face[1]) * 0.055,
        event.at[2] + f64::from(face[2]) * 0.055,
    ];

    let Some(level) = host.level(event.dimension) else {
        return;
    };
    level.play_sound(
        host,
        SND_IMPACT_HIT,
        event.at,
        SoundSource::Players,
        0.78,
        1.18,
    );
    if let Some(block_key) = BLOCK_PARTICLE_KEY.get().copied()
        && let Some(hit_state) = level.block_state(host, event.block)
    {
        let chip_opts = ParticleOptions::simple(block_key);
        level.send_particles(
            host,
            &chip_opts,
            Some(hit_state),
            impact_fx_pos,
            [0.05, 0.05, 0.05],
            0.14,
            10,
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
            4,
        );
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

fn on_bullet_impact_sync(value: BulletImpactSync, cx: &mut dyn IPayloadContext) -> Result<(), String> {
    let block = [value.block_x.0, value.block_y.0, value.block_z.0];
    let face = normalize_cardinal_face([value.face_x.0, value.face_y.0, value.face_z.0]);
    let at = [
        f64::from(block[0]) + f64::from(value.rel_x_dm.0) * 1e-4,
        f64::from(block[1]) + f64::from(value.rel_y_dm.0) * 1e-4,
        f64::from(block[2]) + f64::from(value.rel_z_dm.0) * 1e-4,
    ];

    let light_pos = [block[0] + face[0], block[1] + face[1], block[2] + face[2]];
    let mut light = LightCoords::new(0, 15).unwrap_or(LightCoords::FULL_BRIGHT);
    if let Ok(host) = cx.client_host() {
        if host.block_collision(block).is_empty() {
            return Ok(());
        }
        light = sample_world_light(host, &value.dimension, light_pos);
    }

    let mut fx = world_effects().lock().unwrap();
    if !value.dimension.is_empty() && fx.active_dimension != value.dimension {
        fx.active_dimension = value.dimension.clone();
        fx.holes.clear();
        fx.casings.clear();
    }
    if fx.holes.len() >= MAX_BULLET_HOLES {
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

fn spawn_spent_casing_from_camera(host: &dyn ClientHost, _aim: f32) {
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
        let world_fov =
            f64::from(f32::from_bits(LAST_WORLD_FOV_BITS.load(Ordering::Relaxed)).clamp(20.0, 150.0));
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

    if fx.casings.len() >= MAX_SPENT_CASINGS {
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

fn step_world_effects(host: &dyn ClientHost, dt: f32) {
    let mut clink_sounds: Vec<(f32, f32)> = Vec::new();
    {
        let mut fx = world_effects().lock().unwrap();
        if fx.holes.is_empty() && fx.casings.is_empty() {
            return;
        }

        for hole in &mut fx.holes {
            hole.age += dt;
        }
        fx.holes.retain(|h| h.age < BULLET_HOLE_LIFETIME_SECS);

        fx.prune_timer += dt;
        let refresh_world = fx.prune_timer >= 0.25;
        if refresh_world {
            fx.prune_timer = 0.0;
            fx.holes.retain(|h| !host.block_collision(h.block).is_empty());
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

            for _ in 0..steps {
                casing.vel[1] -= 15.5 * h64;
                let drag = (1.0 - 0.85 * h64).clamp(0.90, 1.0);
                casing.vel[0] *= drag;
                casing.vel[2] *= drag;

                let next_x = casing.pos[0] + casing.vel[0] * h64;
                let next_z = casing.pos[2] + casing.vel[2] * h64;
                if hits_solid_wall(host, next_x, casing.pos[1] + 0.03, casing.pos[2]) {
                    casing.vel[0] = -casing.vel[0] * 0.42;
                } else {
                    casing.pos[0] = next_x;
                }
                if hits_solid_wall(host, casing.pos[0], casing.pos[1] + 0.03, next_z) {
                    casing.vel[2] = -casing.vel[2] * 0.42;
                } else {
                    casing.pos[2] = next_z;
                }

                let prev_y = casing.pos[1];
                let next_y = prev_y + casing.vel[1] * h64;
                if casing.vel[1] <= 0.0
                    && let Some(floor_y) =
                        floor_top_at(host, casing.pos[0], prev_y, casing.pos[2])
                    && next_y <= floor_y + CASING_REST_RADIUS
                {
                    casing.pos[1] = floor_y + CASING_REST_RADIUS;
                    let impact_speed = (-casing.vel[1]) as f32;
                    if impact_speed > 0.58 && casing.bounces < 4 {
                        casing.vel[1] = -casing.vel[1] * 0.38;
                        casing.vel[0] *= 0.62;
                        casing.vel[2] *= 0.62;
                        casing.pitch_deg *= 0.25;
                        casing.pitch_vel *= 0.28;
                        casing.yaw_vel *= 0.55;
                        casing.roll_vel *= 0.65;
                        if casing.bounces < 2 {
                            let vol = if casing.bounces == 0 { 0.72 } else { 0.34 };
                            let pitch_var = 0.94 + 0.12 * fx.next_unit_f32();
                            clink_sounds.push((vol, pitch_var));
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

        casings.retain(|c| c.age < CASING_LIFETIME_SECS && c.pos[1] > -128.0);
        fx.casings = casings;
    }

    for (vol, pitch_mul) in clink_sounds {
        play_local_foley(host, SND_SHELL_DROP, vol, pitch_mul);
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
        [0, 0, 1] => ([1.0_f32, 0.0, 0.0], [0.0_f32, 1.0, 0.0], [0.0_f32, 0.0, 1.0]),
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

fn submit_world_effects(draw: &mut dyn WorldDrawContext) {
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
}
