//! Blackpowder Legacy — Winchester 1873 lever-action rifle (wasm mod, v0.8.1).
//!
//! Controls:
//! - Hold right click: shoulder the rifle and aim down the sights — the cocked
//!   hammer clears the sight line and the iron sights slide onto the camera axis
//!   via frame-rate-independent second-order dynamics (`render_frame_pre` +
//!   `render_hand`), the world FOV zooms smoothly (`compute_fov`), mouse turn
//!   sensitivity scales proportionally (`calculate_player_turn`), and a subtle
//!   vignette frames the view (`gui_render_post`). Disabled while sprinting or
//!   reloading.
//! - Left click (aimed or hipfire while holding the rifle): fire one `.44-40`
//!   round (`key_mapping_pre` cancels vanilla attack/mining and queues a typed
//!   payload). Accuracy rewards aiming (`0.06` ADS, `0.04` crouched) vs hipfire
//!   (`1.50` standing, `2.20` moving). Each live shot applies two-layer recoil
//!   (camera pitch climb with `57%` smooth recovery + micro-jolt, and rear-stock
//!   pivoted 2nd-order weapon kick) followed by the 10-tick lever cycle.
//!   Clicking fire mid-reload immediately interrupts the reload when >= 1 round
//!   is already in the tube.
//! - R (remappable in `Options > Controls`): press once to start a continuous
//!   per-cartridge reload loop (`8t` open -> `10t` per round -> `6t` close) up
//!   to `8 / 8`, or until interrupted by firing, sprinting, or swapping slots.
//!
//! Both first-person player arms (`right_arm` + `left_arm` with sleeves) are
//! extracted from `minecraft:player#main` (with a procedural `LayerDefinition`
//! fallback) and drawn with the unprefixed vanilla atlas key
//! `"entity/player/wide/steve"` under the rifle's exact pose matrix so they
//! follow ADS, recoil, lever action, and per-shell loading-gate insertion in
//! lockstep.

use mod_api::{
    BlockEntityDrawContext, CalculatePlayerTurnEvent, CameraPoseEvent, ClientHost,
    ClientSetupEvent, ClientTickPreEvent, ComputeFovEvent, CubeDeformation, CubeListBuilder,
    EntityUuid, Flow, GuiDraw, GuiRenderPostEvent, Hand, Hooks, Host, HostSide, IPayloadContext,
    Identifier, ItemBarEvent, ItemKey, ItemPart, ItemStack, KeyConflictContext,
    KeyMappingPreEvent, KeyRegistration, LayerDefinition, LevelTickPostEvent, LivingRef,
    MeshDefinition, ModelPart, OnUseTickEvent, PartPose, ParticleKey, ParticleOptions, PayloadId,
    Registrar, RegistrationContext, ReleaseUsingEvent, RenderFramePreEvent, RenderHandEvent,
    ServerHost, ServerTickEvent, ServerTickPhase, ShouldCauseReequipAnimationEvent, SoundSource,
    Text, ToClient, ToServer, UseDurationEvent, VarInt,
};
use std::collections::BTreeMap;
use std::f32::consts::PI;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

const WINCHESTER: &str = "blackpowder_legacy:winchester_1873";
const CARTRIDGE: &str = "blackpowder_legacy:cartridge_44";
const BULLET: &str = "blackpowder_legacy:bullet";

const MOD_ID: &str = "blackpowder_legacy";
const RELOAD_MAPPING: &str = "reload";
const ATTACK_MAPPING: &str = "key.attack";
const INPUT_CHANNEL: &str = "blackpowder_legacy:rifle_input";
const SYNC_CHANNEL: &str = "blackpowder_legacy:rifle_sync";
/// Horizon's sprite atlas registers vanilla sprites without the `"minecraft:"` namespace
/// (`"entity/player/wide/steve"`, matching `Game::first_person_arm` in `frame_geometry.rs`).
const PLAYER_SKIN_WIDE: &str = "entity/player/wide/steve";
const PLAYER_SKIN_WIDE_NS: &str = "minecraft:entity/player/wide/steve";

const TUBE_CAPACITY: u32 = 8;
const USE_DURATION: i32 = 72000;

/// Ticks between consecutive shots while working the lever (`10` ticks = `0.50 s`, `120 RPM`).
const LEVER_TICKS: u64 = 10;
/// Ticks after a shot when the metallic lever-lock click plays (`400 ms`).
const LEVER_CLICK_DELAY_TICKS: u64 = 8;
/// Ticks holding right-click before full server-side ADS accuracy settles.
const ADS_SETTLE_TICKS: i32 = 4;

// Continuous per-cartridge server reload timing (at 20 TPS).
const RELOAD_OPEN_TICKS: u64 = 8;
const RELOAD_INSERT_TICKS: u64 = 10;
const RELOAD_CLOSE_TICKS: u64 = 6;

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

/// `ItemInHandRenderer` right-hand base translation (`equip = 0`, `attack = 0`).
const HAND_BASE: [f32; 3] = [0.56, -0.52, -0.72];

/// First-person right-hand offset placing the cocked Winchester 1873 iron sights
/// (`mira_frontal_ponto` over `mira_traseira_folha` at baked `X = 8.139`) dead-center
/// on screen (`X = 0.0000`) with natural cheek-weld eye relief.
const ADS_OFFSET: [f32; 3] = [-0.5639, 0.3695, 0.05];

/// Mirror of `display.firstperson_righthand` in `models/item/winchester_1873.json`.
/// Vanilla stores item-model translation in sixteenths of a block.
const RIFLE_DISPLAY: ([f32; 3], [f32; 3], f32) = ([0.0, 0.3, -4.75], [0.0, 0.0, 0.0], 0.45);

/// Baked model texel coordinates (`0..16` space) for hand anchors and recoil pivot.
const GRIP_WRIST_TEXELS: [f32; 3] = [8.20, 8.45, 17.20];
const FORESTOCK_TEXELS: [f32; 3] = [7.95, 9.65, 3.20];
const LOADING_GATE_TEXELS: [f32; 3] = [9.05, 10.15, 11.20];
const RECOIL_PIVOT_TEXELS: [f32; 3] = [8.14, 8.60, 22.00];

// Vanilla sound events broadcast through the level (never via commands).
const SHOT_SOUND: &str = "minecraft:entity.firework_rocket.blast";
const DRY_SOUND: &str = "minecraft:block.lever.click";
const LOAD_SOUND: &str = "minecraft:item.crossbow.loading_middle";
const CLICK_SOUND: &str = "minecraft:item.crossbow.loading_end";

const SHOT_VOLUME: f32 = 3.0;
const SHOT_PITCH: f32 = 0.6;
const UI_VOLUME: f32 = 0.7;
const DRY_PITCH: f32 = 0.7;
const LOAD_PITCH: f32 = 1.2;
const READY_PITCH: f32 = 1.0;
const LEVER_PITCH: f32 = 1.5;

const SMOKE_PARTICLE: &str = "minecraft:large_smoke";
const AMMO_BAR_RGB: u32 = 0xC9_A2_27;

// Client -> Server actions.
const ACTION_FIRE: i32 = 1;
const ACTION_RELOAD: i32 = 2;
const ACTION_RELOAD_CANCEL: i32 = 3;

// Server -> Client authoritative weapon feedback events.
const SYNC_SHOT: i32 = 1;
const SYNC_DRY_CLICK: i32 = 2;
const SYNC_RELOAD_START: i32 = 3;
const SYNC_SHELL_INSERTED: i32 = 4;
const SYNC_RELOAD_END: i32 = 5;

// Client-side state.
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
/// Latest `aim_dyn.y` (`f32::to_bits`), updated once per rendered frame in `render_frame_pre`
/// and shared lock-free with `compute_fov`, `calculate_player_turn`, `gui_render_post`, and
/// `render_hand`.
static ADS_BITS: AtomicU32 = AtomicU32::new(0);

// Server-side authoritative state.
static SERVER_TICKS: AtomicU64 = AtomicU64::new(0);
static AIM_TICKS: Mutex<BTreeMap<u128, (i32, u64)>> = Mutex::new(BTreeMap::new());
static LEVER_READY: Mutex<BTreeMap<u128, u64>> = Mutex::new(BTreeMap::new());
static PENDING_LEVER_CLICKS: Mutex<Vec<(u128, u64)>> = Mutex::new(Vec::new());
static RELOAD_SESSIONS: Mutex<BTreeMap<u128, ReloadSession>> = Mutex::new(BTreeMap::new());

static RIFLE_INPUT: OnceLock<ToServer<RifleInput>> = OnceLock::new();
static RIFLE_SYNC: OnceLock<ToClient<RifleSync>> = OnceLock::new();
static RIFLE_KEY: OnceLock<ItemKey> = OnceLock::new();
static SMOKE_KEY: OnceLock<ParticleKey> = OnceLock::new();
static PLAYER_ARMS: OnceLock<PlayerArms> = OnceLock::new();
static CLIENT_ANIM: OnceLock<Mutex<ClientAnimState>> = OnceLock::new();

#[derive(horizon_mod_api_macros::PayloadCodec)]
struct RifleInput {
    action: VarInt,
}

#[derive(horizon_mod_api_macros::PayloadCodec)]
struct RifleSync {
    kind: VarInt,
}

#[derive(Clone, Copy)]
enum ReloadPhase {
    Opening { until: u64 },
    Inserting { next_tick: u64 },
    Closing { until: u64 },
}

#[derive(Clone, Copy)]
struct ReloadSession {
    slot: u32,
    phase: ReloadPhase,
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
    reload_dyn: SecondOrder,
    shot_timer: Option<f32>,
    shot_ads_factor: f32,
    shot_crouch_factor: f32,
    shot_roll_sign: f32,
    cam_recoil_timer: Option<f32>,
    cam_pitch_kick: f32,
    cam_yaw_kick: f32,
    jolt_timer: f32,
    jolt_yaw_sign: f32,
    dry_timer: f32,
    shell_timer: Option<f32>,
    rng: u32,
}

impl ClientAnimState {
    fn new() -> Self {
        Self {
            aim_dyn: SecondOrder::new(3.6, 0.86, 1.05, 0.0),
            recoil_dyn: SecondOrder::new(5.5, 0.58, 2.4, 0.0),
            reload_dyn: SecondOrder::new(3.2, 0.88, 0.80, 0.0),
            shot_timer: None,
            shot_ads_factor: 1.0,
            shot_crouch_factor: 1.0,
            shot_roll_sign: 1.0,
            cam_recoil_timer: None,
            cam_pitch_kick: 0.0,
            cam_yaw_kick: 0.0,
            jolt_timer: 0.0,
            jolt_yaw_sign: 1.0,
            dry_timer: 0.0,
            shell_timer: None,
            rng: 0x6D2B_79F5,
        }
    }

    fn next_unit_f32(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        ((self.rng >> 8) as f32) * (1.0 / 16_777_216.0)
    }

    fn on_shot(&mut self) {
        let aim = self.aim_dyn.y.clamp(0.0, 1.0);
        let sneaking = CLIENT_SNEAKING.load(Ordering::Relaxed);
        let ads_factor = 1.0 - 0.30 * aim;
        let crouch_factor = if sneaking { 0.875 } else { 1.0 };
        let variance = 0.86 + 0.28 * self.next_unit_f32();
        let yaw_rand = self.next_unit_f32() * 2.0 - 1.0;

        self.shot_timer = Some(0.0);
        self.shot_ads_factor = ads_factor;
        self.shot_crouch_factor = crouch_factor;
        self.shot_roll_sign = if yaw_rand >= 0.0 { 1.0 } else { -1.0 };

        self.cam_recoil_timer = Some(0.0);
        self.cam_pitch_kick = 5.4 * ads_factor * crouch_factor * variance;
        self.cam_yaw_kick = 0.72 * ads_factor * yaw_rand;

        self.jolt_timer = 0.065;
        self.jolt_yaw_sign = self.shot_roll_sign;
        self.dry_timer = 0.0;
    }

    fn on_dry_click(&mut self) {
        self.dry_timer = 0.09;
    }
}

struct LimbModel {
    limb: ModelPart,
    detached_sleeve: Option<ModelPart>,
}

struct PlayerArms {
    right: LimbModel,
    left: LimbModel,
}

impl LimbModel {
    fn extract(root: &ModelPart, limb_name: &str, sleeve_name: &str) -> Option<Self> {
        let mut limb = root.get_child(limb_name)?.clone();
        limb.load_pose(PartPose::ZERO);
        // Sleeves sit either as a child of the arm (26.3 PlayerModel) or on root.
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

    fn draw_part_with_skin(draw: &mut dyn BlockEntityDrawContext, part: &ModelPart) {
        if !draw.model_part(part, PLAYER_SKIN_WIDE, 0xFF_FFFF) {
            let _ = draw.model_part(part, PLAYER_SKIN_WIDE_NS, 0xFF_FFFF);
        }
    }

    fn draw(&self, draw: &mut dyn BlockEntityDrawContext) {
        Self::draw_part_with_skin(draw, &self.limb);
        if let Some(sleeve) = &self.detached_sleeve {
            Self::draw_part_with_skin(draw, sleeve);
        }
    }
}

impl PlayerArms {
    /// Extracts `right_arm`/`left_arm` + sleeves from a baked `minecraft:player#main` root.
    fn from_baked_root(root: &ModelPart) -> Option<Self> {
        Some(Self {
            right: LimbModel::extract(root, "right_arm", "right_sleeve")?,
            left: LimbModel::extract(root, "left_arm", "left_sleeve")?,
        })
    }

    /// Procedural 64x64 wide `PlayerModel` arms + `0.25`-inflated sleeves, guaranteeing
    /// non-empty arm geometry even if `bake_layer` returns `None` or `render_hand` runs early.
    fn fallback_wide() -> Self {
        let build_limb = |origin_x: f32, arm_uv: (i32, i32), sleeve_uv: (i32, i32)| -> LimbModel {
            let mut mesh = MeshDefinition::new();
            let root = mesh.root();
            let arm = mesh.add_or_replace_child(
                root,
                "arm",
                CubeListBuilder::create()
                    .tex_offs(arm_uv.0, arm_uv.1)
                    .add_box([origin_x, -2.0, -2.0], [4.0, 12.0, 4.0]),
                PartPose::ZERO,
            );
            mesh.add_or_replace_child(
                arm,
                "sleeve",
                CubeListBuilder::create()
                    .tex_offs(sleeve_uv.0, sleeve_uv.1)
                    .add_box_with(
                        [origin_x, -2.0, -2.0],
                        [4.0, 12.0, 4.0],
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
            right: build_limb(-3.0, (40, 16), (40, 32)),
            left: build_limb(-1.0, (32, 48), (48, 48)),
        }
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

/// Converts baked model texels (`0..16`) into right-hand view space after `RIFLE_DISPLAY`.
fn model_texel_to_hand(tex: [f32; 3]) -> [f32; 3] {
    let (translate, _rot, scale) = RIFLE_DISPLAY;
    let k = scale / 16.0;
    [
        translate[0] / 16.0 + (tex[0] - 8.0) * k,
        translate[1] / 16.0 + (tex[1] - 8.0) * k,
        translate[2] / 16.0 + (tex[2] - 8.0) * k,
    ]
}

/// Cumulative normalized camera recoil curve `C(t)` over `0.00..0.38 s`:
/// `0..95 ms` smooth bell-curve climb to `1.0`, `95..185 ms` apex plateau,
/// `185..380 ms` smooth `57.1%` (`1 / 1.75`) recovery leaving `42.9%` net climb.
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

/// Visual weapon recoil target envelope (`0..0.20 s`): fast `35 ms` snap, smooth `(1 - u)^2` decay.
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

/// Lever-action cycle envelope (`0.13..0.48 s`, matching `LEVER_TICKS = 10`).
fn lever_envelope(t: f32) -> f32 {
    const START: f32 = 0.13;
    const END: f32 = 0.48;
    if !(START..END).contains(&t) {
        return 0.0;
    }
    let u = (t - START) / (END - START);
    (PI * u).sin()
}

/// Draws a single limb anchored by its fist (`local_x = shaft_cx`, `local_y = 9.5/16`)
/// at `fist_pos` so rotations `(rx, ry, rz)` pivot cleanly around the hand's grip point.
fn draw_anchored_limb(
    draw: &mut dyn BlockEntityDrawContext,
    limb: &LimbModel,
    fist_pos: [f32; 3],
    shaft_cx: f32,
    rot_xyz: [f32; 3],
    scale_sxz_sy: [f32; 2],
) {
    let sxz = scale_sxz_sy[0];
    let sy = scale_sxz_sy[1];
    draw.push();
    draw.translate(fist_pos);
    draw.rotate_about([0.0, 1.0, 0.0], rot_xyz[1], [0.0; 3]);
    draw.rotate_about([1.0, 0.0, 0.0], rot_xyz[0], [0.0; 3]);
    draw.rotate_about([0.0, 0.0, 1.0], rot_xyz[2], [0.0; 3]);
    draw.translate([-sxz * shaft_cx, sy * (9.5 / 16.0), 0.0]);
    draw.scale([sxz, -sy, -sxz]);
    limb.draw(draw);
    draw.pop();
}

/// Draws both first-person arms locked to the rifle's exact pose matrix (`render` + `HAND_BASE`).
fn draw_arms(event: &mut RenderHandEvent<'_, '_>, lever: f32, reload_blend: f32, shell_pulse: f32) {
    let arms = PLAYER_ARMS.get_or_init(PlayerArms::fallback_wide);
    let render = &*event.render;
    let translation = render.translation;
    let rotation = render.rotation;
    let scale = render.scale;
    let equip = render.equip;

    let draw = &mut *event.draw;
    draw.push();
    draw.translate(translation);
    draw.rotate_about([1.0, 0.0, 0.0], rotation[0], [0.0; 3]);
    draw.rotate_about([0.0, 1.0, 0.0], rotation[1], [0.0; 3]);
    draw.rotate_about([0.0, 0.0, 1.0], rotation[2], [0.0; 3]);
    draw.scale([scale; 3]);
    draw.translate([HAND_BASE[0], HAND_BASE[1] - equip * 0.6, HAND_BASE[2]]);

    // Right hand holds the stock wrist and swings forward/down with the lever loop.
    let right_tex = [
        GRIP_WRIST_TEXELS[0],
        GRIP_WRIST_TEXELS[1] - 1.10 * lever,
        GRIP_WRIST_TEXELS[2] - 1.80 * lever,
    ];
    draw_anchored_limb(
        draw,
        &arms.right,
        model_texel_to_hand(right_tex),
        -1.0 / 16.0,
        [110.0 - 14.0 * lever, 14.0, -8.0 + 8.0 * lever],
        [0.56, 0.82],
    );

    // Left hand cradles the wooden forestock and slides back to feed shells into `porta_B`.
    let gate_mix = reload_blend.clamp(0.0, 1.0);
    let left_tex = [
        FORESTOCK_TEXELS[0]
            + (LOADING_GATE_TEXELS[0] - FORESTOCK_TEXELS[0]) * gate_mix
            - 0.38 * shell_pulse,
        FORESTOCK_TEXELS[1]
            + (LOADING_GATE_TEXELS[1] - FORESTOCK_TEXELS[1]) * gate_mix
            + 0.28 * shell_pulse,
        FORESTOCK_TEXELS[2] + (LOADING_GATE_TEXELS[2] - FORESTOCK_TEXELS[2]) * gate_mix,
    ];
    draw_anchored_limb(
        draw,
        &arms.left,
        model_texel_to_hand(left_tex),
        1.0 / 16.0,
        [
            112.0 - 10.0 * gate_mix + 4.0 * shell_pulse,
            -24.0 + 14.0 * gate_mix - 5.0 * shell_pulse,
            18.0 + 12.0 * gate_mix,
        ],
        [0.56, 1.30],
    );

    draw.pop();
}

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let winchester = r.item_key(WINCHESTER);
        let smoke = r.particle_key(SMOKE_PARTICLE);
        r.item_part(winchester, Winchester { smoke });
        let _ = RIFLE_KEY.set(winchester);
        let _ = SMOKE_KEY.set(r.particle_key(SMOKE_PARTICLE));

        r.register_key_mapping(
            KeyRegistration::new(RELOAD_MAPPING, "key.keyboard.r")
                .label("key.blackpowder_legacy.reload")
                .context(KeyConflictContext::InGame),
        );

        let mut payloads = r.payload_handlers();
        let mut view = payloads.registrar("1");
        let input = view.play_to_server_typed::<RifleInput>(
            PayloadId(INPUT_CHANNEL.into()),
            on_rifle_input,
        );
        let sync = view.play_to_client_typed::<RifleSync>(
            PayloadId(SYNC_CHANNEL.into()),
            on_rifle_sync,
        );
        let _ = RIFLE_INPUT.set(input);
        let _ = RIFLE_SYNC.set(sync);
    }

    fn client_setup(&self, host: &mut dyn ClientHost, _: &mut ClientSetupEvent) {
        let arms = host
            .bake_layer("minecraft:player#main")
            .and_then(|root| PlayerArms::from_baked_root(&root))
            .unwrap_or_else(PlayerArms::fallback_wide);
        let _ = PLAYER_ARMS.set(arms);
    }

    /// Left click while holding or aiming the rifle fires a round (`Flow::Cancel` suppresses
    /// vanilla mining and melee punches).
    fn key_mapping_pre(&self, host: &mut dyn ClientHost, event: &mut KeyMappingPreEvent<'_>) -> Flow {
        let holding = holding_rifle_recently() || AIMING.load(Ordering::Relaxed);
        if event.mapping == ATTACK_MAPPING && event.pressed && holding && host.playing() {
            PENDING_FIRE.store(true, Ordering::Relaxed);
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

        let holding = holding_rifle_recently() || AIMING.load(Ordering::Relaxed);
        let Some(input) = RIFLE_INPUT.get() else {
            return;
        };

        if !host.playing() || !holding {
            PENDING_FIRE.store(false, Ordering::Relaxed);
            CLIENT_SPRINTING.store(false, Ordering::Relaxed);
            CLIENT_SNEAKING.store(false, Ordering::Relaxed);
            if !holding && CLIENT_RELOADING.swap(false, Ordering::Relaxed) {
                let _ = input.send(
                    host,
                    &RifleInput {
                        action: VarInt(ACTION_RELOAD_CANCEL),
                    },
                );
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
        }

        if CLIENT_RELOADING.load(Ordering::Relaxed) && AIMING.swap(false, Ordering::Relaxed) {
            let _ = host.set_mapping_down("key.use", false);
        }

        let mut want_reload = false;
        while host.consume_click(MOD_ID, RELOAD_MAPPING) {
            want_reload = true;
        }

        if PENDING_RELOAD_CANCEL.swap(false, Ordering::Relaxed) {
            let _ = input.send(
                host,
                &RifleInput {
                    action: VarInt(ACTION_RELOAD_CANCEL),
                },
            );
        }

        if PENDING_FIRE.swap(false, Ordering::Relaxed) {
            let _ = input.send(
                host,
                &RifleInput {
                    action: VarInt(ACTION_FIRE),
                },
            );
        }

        if want_reload && !CLIENT_RELOADING.load(Ordering::Relaxed) && !sprinting {
            AIMING.store(false, Ordering::Relaxed);
            let _ = host.set_mapping_down("key.use", false);
            let _ = input.send(
                host,
                &RifleInput {
                    action: VarInt(ACTION_RELOAD),
                },
            );
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
        tick_pending_lever_clicks(host, now);
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

        let holding = holding_rifle_recently() || AIMING.load(Ordering::Relaxed);
        let aiming_want = holding
            && AIMING.load(Ordering::Relaxed)
            && !CLIENT_RELOADING.load(Ordering::Relaxed)
            && !CLIENT_SPRINTING.load(Ordering::Relaxed);
        let reload_want = holding && CLIENT_RELOADING.load(Ordering::Relaxed);

        let mut cam_delta_pitch = 0.0_f32;
        let mut cam_delta_yaw = 0.0_f32;

        {
            let mut anim = client_anim().lock().unwrap();
            if !holding {
                anim.aim_dyn.reset(0.0);
                anim.recoil_dyn.reset(0.0);
                anim.reload_dyn.reset(0.0);
                anim.shot_timer = None;
                anim.cam_recoil_timer = None;
                anim.jolt_timer = 0.0;
                anim.dry_timer = 0.0;
                anim.shell_timer = None;
                ADS_BITS.store(0.0_f32.to_bits(), Ordering::Relaxed);
                return;
            }

            let aim_y = anim.aim_dyn.step(dt, if aiming_want { 1.0 } else { 0.0 });
            ADS_BITS.store(aim_y.clamp(0.0, 1.2).to_bits(), Ordering::Relaxed);

            let _ = anim.reload_dyn.step(dt, if reload_want { 1.0 } else { 0.0 });

            let kick_target = if let Some(cur) = anim.shot_timer {
                let next = cur + dt;
                anim.shot_timer = if next >= 0.52 { None } else { Some(next) };
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
            if anim.dry_timer > 0.0 {
                anim.dry_timer = (anim.dry_timer - dt).max(0.0);
            }
            if let Some(st) = &mut anim.shell_timer {
                *st += dt;
                if *st >= 0.24 {
                    anim.shell_timer = None;
                }
            }
        }

        if (cam_delta_pitch.abs() > 1e-5 || cam_delta_yaw.abs() > 1e-5)
            && let Some(cam) = host.camera_entity()
            && let Some([yaw, pitch]) = cam.rotation(host)
        {
            // Negative pitch looks up in Minecraft; upward recoil subtracts from pitch.
            let new_pitch = (pitch - cam_delta_pitch).clamp(-89.9, 89.9);
            let new_yaw = yaw + cam_delta_yaw;
            let _ = host.set_local_rotation(new_yaw, new_pitch);
        }
    }

    /// World FOV zoom per rendered frame (`ViewportEvent.ComputeFov`), synchronized with `ADS_BITS`.
    fn compute_fov(&self, _: &dyn ClientHost, event: &mut ComputeFovEvent) {
        let aim = aim_progress().clamp(0.0, 1.15);
        if aim > 0.001 {
            let scale = 1.0 - (1.0 - AIM_FOV) * aim;
            let scaled = (event.degrees * scale).clamp(1.0, 179.0);
            if scaled.is_finite() {
                event.degrees = scaled;
            }
        }
    }

    /// Proportional mouse sensitivity scaling while aiming down sights.
    fn calculate_player_turn(&self, _: &dyn ClientHost, event: &mut CalculatePlayerTurnEvent) {
        let aim = aim_progress().clamp(0.0, 1.0);
        if aim > 0.001 {
            let sens_scale = 1.0 - (1.0 - AIM_FOV) * aim * 0.85;
            event.mouse_sensitivity *= f64::from(sens_scale);
        }
    }

    /// Transient high-frequency shot micro-jolt (`65 ms`).
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

    fn render_hand(&self, _: &dyn ClientHost, event: &mut RenderHandEvent<'_, '_>) -> Flow {
        let Some(rifle) = RIFLE_KEY.get().copied() else {
            return Flow::Continue;
        };
        {
            let render = &mut *event.render;
            if render.hand != Hand::Main {
                return Flow::Continue;
            }
            if !matches!(render.stack.map(|stack| stack.item()), Some(item) if item == rifle) {
                RIFLE_SEEN_FRAME.store(0, Ordering::Relaxed);
                return Flow::Continue;
            }
        }
        RIFLE_SEEN_FRAME.store(FRAME_SEQ.load(Ordering::Relaxed), Ordering::Relaxed);

        let (
            aim,
            kick,
            reload_blend,
            shot_t,
            ads_factor,
            crouch_factor,
            roll_sign,
            dry_t,
            shell_t,
        ) = {
            let anim = client_anim().lock().unwrap();
            (
                anim.aim_dyn.y,
                anim.recoil_dyn.y,
                anim.reload_dyn.y.clamp(0.0, 1.15),
                anim.shot_timer,
                anim.shot_ads_factor,
                anim.shot_crouch_factor,
                anim.shot_roll_sign,
                anim.dry_timer,
                anim.shell_timer,
            )
        };

        let lever = shot_t.map_or(0.0, lever_envelope);
        let shell_pulse = shell_t.map_or(0.0, |t| (PI * (t / 0.24).clamp(0.0, 1.0)).sin());

        {
            let render = &mut *event.render;
            let aim_clamped = aim.clamp(0.0, 1.0);
            // Natural shoulder-pocket parabolic dip while raising/lowering sights.
            let arc_dip = -0.032 * (4.0 * aim_clamped * (1.0 - aim_clamped));
            render.translation[0] += ADS_OFFSET[0] * aim;
            render.translation[1] += ADS_OFFSET[1] * aim + arc_dip;
            render.translation[2] += ADS_OFFSET[2] * aim;
            render.rotation[2] += 3.2 * (1.0 - aim_clamped) * (1.0 - reload_blend.clamp(0.0, 1.0));

            // Rear-stock-pivoted 2nd-order visual recoil kick.
            if kick.abs() > 1e-4 {
                let pitch_deg = 7.5 * kick * ads_factor * crouch_factor;
                let pitch_rad = pitch_deg * (PI / 180.0);
                let pivot = model_texel_to_hand(RECOIL_PIVOT_TEXELS);
                let py = HAND_BASE[1] - render.equip * 0.6 + pivot[1];
                let pz = HAND_BASE[2] + pivot[2];
                let dy = py - (py * pitch_rad.cos() - pz * pitch_rad.sin());
                let dz = pz - (py * pitch_rad.sin() + pz * pitch_rad.cos());

                render.translation[1] += dy + 0.008 * kick * ads_factor;
                render.translation[2] += dz + 0.105 * kick * ads_factor;
                render.rotation[0] += pitch_deg;
                render.rotation[1] += 0.9 * kick * ads_factor * roll_sign;
                render.rotation[2] += 1.4 * kick * ads_factor * roll_sign;
            }

            // Lever-action cycle between shots (`0.13..0.48 s`).
            if lever > 1e-4 {
                let damp = 1.0 - 0.35 * aim_clamped;
                render.translation[1] -= 0.042 * lever * damp;
                render.translation[2] += 0.018 * lever * damp;
                render.rotation[0] -= 3.2 * lever * damp;
                render.rotation[2] += 11.0 * lever * damp;
            }

            // Empty-chamber dry click flinch.
            if dry_t > 0.0 {
                let flinch = (PI * (dry_t / 0.09).clamp(0.0, 1.0)).sin();
                render.translation[2] += 0.010 * flinch;
                render.rotation[0] -= 0.9 * flinch;
            }

            // Continuous loading-gate tilt + per-shell insertion seat pulse.
            if reload_blend > 1e-3 || shell_pulse > 1e-3 {
                render.translation[0] -= 0.048 * reload_blend;
                render.translation[1] -= 0.042 * reload_blend + 0.016 * shell_pulse;
                render.translation[2] += 0.038 * reload_blend + 0.012 * shell_pulse;
                render.rotation[0] += 5.5 * reload_blend + 2.8 * shell_pulse;
                render.rotation[1] += 8.5 * reload_blend;
                render.rotation[2] -= 14.5 * reload_blend - 3.5 * shell_pulse;
            }
        }

        draw_arms(event, lever, reload_blend, shell_pulse);
        Flow::Continue
    }

    fn gui_render_post(&self, _: &dyn ClientHost, event: &mut GuiRenderPostEvent<'_>) {
        let aim = aim_progress().clamp(0.0, 1.0);
        if aim < 0.02 || event.gui_hidden {
            return;
        }
        let [w, h] = event.batch.size();
        let brightness = ((1.0 - (1.0 - AIM_VIGNETTE) * aim) * 255.0).clamp(0.0, 255.0) as u32;
        let color = 0xFF00_0000 | (brightness << 16) | (brightness << 8) | brightness;
        let (w, h) = (w as i32, h as i32);
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
}

fn on_rifle_input(value: RifleInput, cx: &mut dyn IPayloadContext) -> Result<(), String> {
    let player = cx.player().map_err(|e| format!("{e:?}"))?;
    let host = cx.server_host().map_err(|e| format!("{e:?}"))?;
    match value.action.0 {
        ACTION_FIRE => {
            if let Some(smoke) = SMOKE_KEY.get().copied() {
                fire_rifle(host, player, smoke);
            }
        }
        ACTION_RELOAD => start_reload(host, player),
        ACTION_RELOAD_CANCEL => {
            let _ = cancel_reload_session(host, player);
        }
        _ => {}
    }
    Ok(())
}

fn on_rifle_sync(value: RifleSync, _: &mut dyn IPayloadContext) -> Result<(), String> {
    match value.kind.0 {
        SYNC_SHOT => {
            CLIENT_RELOADING.store(false, Ordering::Relaxed);
            client_anim().lock().unwrap().on_shot();
        }
        SYNC_DRY_CLICK => {
            client_anim().lock().unwrap().on_dry_click();
        }
        SYNC_RELOAD_START => {
            CLIENT_RELOADING.store(true, Ordering::Relaxed);
            AIMING.store(false, Ordering::Relaxed);
        }
        SYNC_SHELL_INSERTED => {
            CLIENT_RELOADING.store(true, Ordering::Relaxed);
            client_anim().lock().unwrap().shell_timer = Some(0.0);
        }
        SYNC_RELOAD_END => {
            CLIENT_RELOADING.store(false, Ordering::Relaxed);
        }
        _ => {}
    }
    Ok(())
}

fn send_sync(host: &mut dyn ServerHost, player: mod_api::PlayerRef, kind: i32) {
    if let Some(sync) = RIFLE_SYNC.get() {
        let _ = sync.send(host, player, &RifleSync { kind: VarInt(kind) });
    }
}

struct Winchester {
    smoke: ParticleKey,
}

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
        send_sync(host, player, SYNC_RELOAD_END);
    }
    removed
}

fn play_sound_at(
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

fn smoke_at_muzzle(host: &mut dyn ServerHost, player: mod_api::PlayerRef, smoke: ParticleKey) {
    let entity = player.entity();
    let Some(level) = entity.level(host) else {
        return;
    };
    let Some(eye) = entity.eye_position(host) else {
        return;
    };
    let Some(view) = entity.view_vector(host) else {
        return;
    };
    let muzzle = [
        eye[0] + view[0] * 0.7,
        eye[1] + view[1] * 0.7 - 0.1,
        eye[2] + view[2] * 0.7,
    ];
    let options = ParticleOptions::simple(smoke);
    level.send_particles(host, &options, None, muzzle, [0.05, 0.05, 0.05], 0.02, 8);
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

fn fire_rifle(host: &mut dyn ServerHost, player: mod_api::PlayerRef, smoke: ParticleKey) {
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
            play_sound_at(host, player, DRY_SOUND, UI_VOLUME, DRY_PITCH);
            send_sync(host, player, SYNC_DRY_CLICK);
            return;
        }
    }

    let lever_ready = LEVER_READY.lock().unwrap().get(&uuid).copied().unwrap_or(0);
    if lever_ready > now {
        return;
    }

    if loaded == 0 {
        play_sound_at(host, player, DRY_SOUND, UI_VOLUME, DRY_PITCH);
        send_sync(host, player, SYNC_DRY_CLICK);
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

    LEVER_READY.lock().unwrap().insert(uuid, now + LEVER_TICKS);
    if remaining > 0 {
        PENDING_LEVER_CLICKS
            .lock()
            .unwrap()
            .push((uuid, now + LEVER_CLICK_DELAY_TICKS));
    }

    play_sound_at(host, player, SHOT_SOUND, SHOT_VOLUME, SHOT_PITCH);
    smoke_at_muzzle(host, player, smoke);
    let _ = player.action_bar(
        host,
        Text::literal(format!("{remaining} / {TUBE_CAPACITY}")),
    );
    send_sync(host, player, SYNC_SHOT);
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
        let _ = player.action_bar(host, Text::literal(format!("{loaded} / {TUBE_CAPACITY}")));
        return;
    }

    let infinite = player.has_infinite_materials(host).unwrap_or(false);
    if !infinite && !has_cartridge(host, player) {
        play_sound_at(host, player, DRY_SOUND, UI_VOLUME, DRY_PITCH);
        let _ = player.action_bar(host, Text::literal(format!("{loaded} / {TUBE_CAPACITY}")));
        return;
    }

    AIM_TICKS.lock().unwrap().remove(&uuid);
    let now = SERVER_TICKS.load(Ordering::Relaxed);
    RELOAD_SESSIONS.lock().unwrap().insert(
        uuid,
        ReloadSession {
            slot,
            phase: ReloadPhase::Opening {
                until: now + RELOAD_OPEN_TICKS,
            },
        },
    );
    play_sound_at(host, player, CLICK_SOUND, UI_VOLUME * 0.85, LEVER_PITCH);
    send_sync(host, player, SYNC_RELOAD_START);
}

fn tick_pending_lever_clicks(host: &mut dyn ServerHost, now: u64) {
    let mut due = Vec::new();
    {
        let mut list = PENDING_LEVER_CLICKS.lock().unwrap();
        let mut i = 0;
        while i < list.len() {
            let (uuid, tick_at) = list[i];
            if now >= tick_at {
                if host.entity_by_uuid(EntityUuid(uuid)).is_some() {
                    due.push(uuid);
                    list.swap_remove(i);
                    continue;
                } else if now.saturating_sub(tick_at) > 40 {
                    list.swap_remove(i);
                    continue;
                }
            }
            i += 1;
        }
    }
    for uuid in due {
        if let Some(player) = host.player(uuid)
            && held_rifle(host, player).is_some()
        {
            play_sound_at(host, player, CLICK_SOUND, UI_VOLUME, LEVER_PITCH);
        }
    }
}

fn tick_reload_sessions(host: &mut dyn ServerHost, now: u64) {
    let uuids: Vec<u128> = RELOAD_SESSIONS.lock().unwrap().keys().copied().collect();
    for uuid in uuids {
        // Process only sessions whose player belongs to the server level currently ticking.
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
            send_sync(host, player, SYNC_RELOAD_END);
            continue;
        };
        if sprinting {
            RELOAD_SESSIONS.lock().unwrap().remove(&uuid);
            send_sync(host, player, SYNC_RELOAD_END);
            continue;
        }

        let mut finished = false;
        match session.phase {
            ReloadPhase::Opening { until } => {
                if now >= until {
                    session.phase = ReloadPhase::Inserting { next_tick: now };
                }
            }
            ReloadPhase::Inserting { next_tick } => {
                if now >= next_tick {
                    let loaded = rounds(&rifle);
                    let infinite = player.has_infinite_materials(host).unwrap_or(false);
                    if loaded >= TUBE_CAPACITY {
                        session.phase = ReloadPhase::Closing {
                            until: now + RELOAD_CLOSE_TICKS,
                        };
                    } else if infinite || take_cartridge(host, player) {
                        let next_loaded = loaded + 1;
                        let mut rifle = rifle;
                        rifle.set_damage(next_loaded);
                        if player.set_inventory_slot(host, slot, Some(rifle)).is_err() {
                            finished = true;
                        } else {
                            if next_loaded >= TUBE_CAPACITY {
                                play_sound_at(host, player, CLICK_SOUND, UI_VOLUME, READY_PITCH);
                                session.phase = ReloadPhase::Closing {
                                    until: now + RELOAD_CLOSE_TICKS,
                                };
                            } else {
                                play_sound_at(host, player, LOAD_SOUND, UI_VOLUME, LOAD_PITCH);
                                if infinite || has_cartridge(host, player) {
                                    session.phase = ReloadPhase::Inserting {
                                        next_tick: now + RELOAD_INSERT_TICKS,
                                    };
                                } else {
                                    session.phase = ReloadPhase::Closing {
                                        until: now + RELOAD_CLOSE_TICKS,
                                    };
                                }
                            }
                            let _ = player.action_bar(
                                host,
                                Text::literal(format!("{next_loaded} / {TUBE_CAPACITY}")),
                            );
                            send_sync(host, player, SYNC_SHELL_INSERTED);
                        }
                    } else {
                        session.phase = ReloadPhase::Closing {
                            until: now + RELOAD_CLOSE_TICKS,
                        };
                    }
                }
            }
            ReloadPhase::Closing { until } => {
                if now >= until {
                    finished = true;
                }
            }
        }

        if finished {
            RELOAD_SESSIONS.lock().unwrap().remove(&uuid);
            LEVER_READY.lock().unwrap().insert(uuid, now + 2);
            send_sync(host, player, SYNC_RELOAD_END);
        } else {
            RELOAD_SESSIONS.lock().unwrap().insert(uuid, session);
        }
    }
}

#[mod_api::horizon_part]
impl ItemPart for Winchester {
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
                let mut ticks = AIM_TICKS.lock().unwrap();
                let is_fresh = ticks
                    .get(&uuid)
                    .is_none_or(|&(_, prev)| now.saturating_sub(prev) > 3);
                ticks.insert(uuid, (held, now));
                drop(ticks);
                if is_fresh
                    && let Some((_, rifle)) = held_rifle(host, player)
                {
                    let loaded = rounds(&rifle);
                    let _ = player.action_bar(
                        host,
                        Text::literal(format!("{loaded} / {TUBE_CAPACITY}")),
                    );
                }
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
