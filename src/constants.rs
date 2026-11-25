//! Content IDs, gameplay & ballistics parameters, and Blockbench `[0..16]` texel pivots.

// ---------------------------------------------------------------------------
// Content & Asset Identifiers
// ---------------------------------------------------------------------------

pub(crate) const MOD_ID: &str = "just_a_rifle";
pub(crate) const WINCHESTER: &str = "just_a_rifle:winchester_1873";
pub(crate) const CARTRIDGE: &str = "just_a_rifle:cartridge_44";
pub(crate) const BULLET: &str = "just_a_rifle:bullet";

pub(crate) const MODEL_BODY_ID: &str = "just_a_rifle:special/winchester_body";
pub(crate) const MODEL_BODY_LOD_ID: &str = "just_a_rifle:special/winchester_body_lod";
pub(crate) const MODEL_LEVER_ID: &str = "just_a_rifle:special/winchester_lever";
pub(crate) const MODEL_HAMMER_ID: &str = "just_a_rifle:special/winchester_hammer";
pub(crate) const MODEL_TRIGGER_ID: &str = "just_a_rifle:special/winchester_trigger";
pub(crate) const MODEL_GATE_ID: &str = "just_a_rifle:special/winchester_gate";
pub(crate) const MODEL_CARTRIDGE_ID: &str = "just_a_rifle:special/winchester_cartridge";
pub(crate) const MODEL_CASING_ID: &str = "just_a_rifle:special/winchester_casing";
pub(crate) const MODEL_BULLET_HOLE_ID: &str = "just_a_rifle:special/bullet_hole";

pub(crate) const SND_FIRE: &str = "just_a_rifle:item.winchester_fire";
pub(crate) const SND_THUMP: &str = "just_a_rifle:item.winchester_thump";
pub(crate) const SND_LEVER: &str = "just_a_rifle:item.winchester_lever";
pub(crate) const SND_COCK: &str = "just_a_rifle:item.winchester_cock";
pub(crate) const SND_RELOAD_INTRO: &str = "just_a_rifle:animation.winchester_reload_intro";
pub(crate) const SND_RELOAD_INSERT: &str = "just_a_rifle:animation.winchester_reload_insert";
pub(crate) const SND_RELOAD_END: &str = "just_a_rifle:animation.winchester_reload_end";
pub(crate) const SND_DRAW: &str = "just_a_rifle:animation.winchester_draw";
pub(crate) const SND_INSPECT: &str = "just_a_rifle:animation.winchester_inspect";
pub(crate) const SND_SHELL_DROP: &str = "just_a_rifle:item.winchester_shell_drop";
pub(crate) const SND_MISFIRE_WET: &str = "just_a_rifle:item.winchester_misfire_wet";
pub(crate) const SND_IMPACT_HIT: &str = "minecraft:block.stone.hit";
pub(crate) const SND_RICOCHET_METAL_HEAVY: &str = "minecraft:block.chain.break";
pub(crate) const SND_RICOCHET_METAL_SHEET: &str = "minecraft:block.lantern.break";
pub(crate) const SND_RICOCHET_STONE_CHIP: &str = "minecraft:block.chain.hit";
pub(crate) const SND_CRYSTAL_CHIME: &str = "minecraft:block.amethyst_block.chime";
pub(crate) const SND_BELL_USE: &str = "minecraft:block.bell.use";
pub(crate) const SND_TARGET_BULLSEYE: &str = "minecraft:block.note_block.chime";
pub(crate) const SND_CANDLE_EXTINGUISH: &str = "minecraft:block.candle.extinguish";
pub(crate) const SND_WATER_SPLASH: &str = "minecraft:entity.fishing_bobber.splash";
pub(crate) const SND_WATER_BUBBLES: &str = "minecraft:block.bubble_column.upwards_inside";
pub(crate) const SND_HIT_CONFIRM_SUBTLE: &str = "minecraft:entity.arrow.hit";
pub(crate) const SND_HIT_CONFIRM_CHIME: &str = "minecraft:block.note_block.bell";

pub(crate) const RELOAD_MAPPING: &str = "reload";
pub(crate) const INSPECT_MAPPING: &str = "inspect";
pub(crate) const ATTACK_MAPPING: &str = "key.attack";

pub(crate) const INPUT_CHANNEL: &str = "just_a_rifle:rifle_input";
pub(crate) const SYNC_CHANNEL: &str = "just_a_rifle:rifle_sync";
pub(crate) const SKIN_CHANNEL: &str = "just_a_rifle:rifle_skin";
pub(crate) const IMPACT_CHANNEL: &str = "just_a_rifle:bullet_impact";

pub(crate) const SMOKE_PARTICLE: &str = "minecraft:smoke";
pub(crate) const WHITE_SMOKE_PARTICLE: &str = "minecraft:white_smoke";
pub(crate) const FLAME_PARTICLE: &str = "minecraft:small_flame";
pub(crate) const POOF_PARTICLE: &str = "minecraft:poof";
pub(crate) const CRIT_PARTICLE: &str = "minecraft:crit";
pub(crate) const BLOCK_PARTICLE: &str = "minecraft:block";
pub(crate) const BUBBLE_PARTICLE: &str = "minecraft:bubble";
pub(crate) const BUBBLE_POP_PARTICLE: &str = "minecraft:bubble_pop";
pub(crate) const SPLASH_PARTICLE: &str = "minecraft:splash";
pub(crate) const WATER_FLUID: &str = "minecraft:water";
pub(crate) const FLOWING_WATER_FLUID: &str = "minecraft:flowing_water";
pub(crate) const BELL_BLOCK: &str = "minecraft:bell";
pub(crate) const TARGET_BLOCK: &str = "minecraft:target";
pub(crate) const DECORATED_POT_BLOCK: &str = "minecraft:decorated_pot";
pub(crate) const CHORUS_FLOWER_BLOCK: &str = "minecraft:chorus_flower";
pub(crate) const POINTED_DRIPSTONE_BLOCK: &str = "minecraft:pointed_dripstone";
pub(crate) const TAG_FLOWER_POTS: &str = "minecraft:flower_pots";
pub(crate) const TAG_CANDLES: &str = "minecraft:candles";
pub(crate) const GAME_EVENT_BLOCK_CHANGE: &str = "minecraft:block_change";
pub(crate) const RULE_PROJECTILES_BREAK_BLOCKS: &str = "projectiles_can_break_blocks";
pub(crate) const RULE_BLOCK_DROPS: &str = "block_drops";
pub(crate) const DEFAULT_DIMENSION: &str = "minecraft:overworld";

pub(crate) const LOOT_ABANDONED_MINESHAFT: &str = "minecraft:chests/abandoned_mineshaft";
pub(crate) const LOOT_PILLAGER_OUTPOST: &str = "minecraft:chests/pillager_outpost";
pub(crate) const LOOT_WOODLAND_MANSION: &str = "minecraft:chests/woodland_mansion";
pub(crate) const LOOT_SIMPLE_DUNGEON: &str = "minecraft:chests/simple_dungeon";
pub(crate) const LOOT_DESERT_PYRAMID: &str = "minecraft:chests/desert_pyramid";

pub(crate) const TRIG_FRONTIER_RELIC: &str = "just_a_rifle:frontier_relic";
pub(crate) const TRIG_FULL_TUBE: &str = "just_a_rifle:full_tube";
pub(crate) const TRIG_WET_POWDER: &str = "just_a_rifle:wet_powder";
pub(crate) const TRIG_BELL_RINGER: &str = "just_a_rifle:bell_ringer";
pub(crate) const TRIG_CANDLE_SNUFFER: &str = "just_a_rifle:candle_snuffer";
pub(crate) const TRIG_DEADEYE: &str = "just_a_rifle:deadeye_bullseye";
pub(crate) const TRIG_SHERIFF: &str = "just_a_rifle:sheriff";
pub(crate) const TRIG_LONG_SHOT: &str = "just_a_rifle:long_shot";

pub(crate) const ADV_BELL_MIN_DIST: f64 = 30.0;
pub(crate) const ADV_DEADEYE_MIN_DIST: f64 = 50.0;
pub(crate) const ADV_LONG_SHOT_MIN_DIST: f64 = 60.0;

pub(crate) const MAX_BULLET_HOLES: usize = 96;
pub(crate) const BULLET_HOLE_LIFETIME_SECS: f32 = 35.0;
pub(crate) const BULLET_HOLE_BASE_SCALE: f32 = 0.44;
pub(crate) const BULLET_HOLE_MIN_FACE_SPAN: f64 = 0.36;
pub(crate) const MAX_SPENT_CASINGS: usize = 48;
pub(crate) const CASING_LIFETIME_SECS: f32 = 20.0;
pub(crate) const CASING_BASE_SCALE: f32 = 0.64;
pub(crate) const CASING_REST_RADIUS: f64 = 0.012;
pub(crate) const CASING_WATER_GRAVITY: f64 = 3.2;
pub(crate) const CASING_WATER_MAX_SINK: f64 = -1.15;
pub(crate) const CASING_RISE_START_SECS: f32 = LEVER_DELAY_SECS + 0.06;
pub(crate) const CASING_HANDOFF_SECS: f32 = LEVER_DELAY_SECS + 0.16;
pub(crate) const CASING_LAST_RISE_START_SECS: f32 = 0.06;
pub(crate) const CASING_LAST_HANDOFF_SECS: f32 = 0.15;
pub(crate) const AMMO_BAR_RGB: u32 = 0xC9_A2_27;
pub(crate) const DEFAULT_STEVE_WIDE: &str = "entity/player/wide/steve";

// ---------------------------------------------------------------------------
// Network Action & Sync Discriminants
// ---------------------------------------------------------------------------

pub(crate) const ACTION_FIRE: i32 = 1;
pub(crate) const ACTION_RELOAD: i32 = 2;
pub(crate) const ACTION_RELOAD_CANCEL: i32 = 3;
pub(crate) const ACTION_SKIN_REQUEST: i32 = 4;

pub(crate) const SYNC_SHOT: i32 = 1;
pub(crate) const SYNC_DRY_CLICK: i32 = 2;
pub(crate) const SYNC_RELOAD_START: i32 = 3;
pub(crate) const SYNC_SHELL_INSERTED: i32 = 4;
pub(crate) const SYNC_RELOAD_END_NORM: i32 = 5;
pub(crate) const SYNC_RELOAD_END_EMPTY: i32 = 6;
pub(crate) const SYNC_RELOAD_ABORTED: i32 = 7;
pub(crate) const SYNC_WET_MISFIRE: i32 = 8;
pub(crate) const SYNC_HIT_CONFIRM: i32 = 9;

// ---------------------------------------------------------------------------
// Gameplay & Ballistics Parameters
// ---------------------------------------------------------------------------

pub(crate) const TUBE_CAPACITY: u32 = 8;
pub(crate) const USE_DURATION: i32 = 72_000;

/// 10-tick lever action cycle (`120 RPM`) + 3-tick hammer/recoil settle = 13 ticks (`0.65 s`).
pub(crate) const LEVER_TICKS: u64 = 13;
pub(crate) const LEVER_DELAY_SECS: f32 = 0.14;
pub(crate) const LEVER_DURATION_SECS: f32 = 0.48;

/// Ticks of continuous right-click hold until full ADS accuracy (`0.20 s`).
pub(crate) const ADS_SETTLE_TICKS: i32 = 4;

/// Per-round side-gate reload timings (`ticks @ 20 Hz`).
pub(crate) const RELOAD_OPEN_TICKS: u64 = 8;
pub(crate) const RELOAD_INSERT_TICKS: u64 = 10;
pub(crate) const RELOAD_CLOSE_NORM_TICKS: u64 = 7;
pub(crate) const RELOAD_CLOSE_EMPTY_TICKS: u64 = 15;

/// Client-side animation durations (`seconds`).
pub(crate) const DRAW_DURATION_SECS: f32 = 0.48;
pub(crate) const INSPECT_DURATION_SECS: f32 = 3.40;

/// Projectile speed (`blocks / tick` -> `300 m/s` at 20 TPS) and stance spread (`degrees`).
pub(crate) const BULLET_SPEED: f32 = 15.0;
/// Maximum underwater travel (`blocks`) on the tick a `300 m/s` `.44-40` lead slug crosses
/// from air into water, followed by strong quadratic/exponential hydrodynamic drag (`~3.4m` total).
pub(crate) const WATER_FIRST_TICK_PENETRATION: f64 = 2.10;
pub(crate) const WATER_BULLET_DRAG: f64 = 0.42;
pub(crate) const WATER_BULLET_STOP_SPEED: f64 = 0.18;
pub(crate) const SPREAD_ADS_CROUCH: f32 = 0.04;
pub(crate) const SPREAD_ADS_STAND: f32 = 0.06;
pub(crate) const SPREAD_ADS_MOVING: f32 = 0.12;
pub(crate) const SPREAD_HIP_CROUCH: f32 = 1.10;
pub(crate) const SPREAD_HIP_STAND: f32 = 1.50;
pub(crate) const SPREAD_HIP_MOVING: f32 = 2.20;

/// FOV multiplier at full ADS (`0.67` = ~1.5x iron-sight zoom).
pub(crate) const AIM_FOV: f32 = 0.67;
/// Screen-edge vignette brightness at full ADS (`1.0` = none, `0.68` = subtle framing).
pub(crate) const AIM_VIGNETTE: f32 = 0.68;

// ---------------------------------------------------------------------------
// Exact Blockbench `[0..16]` JSON Texel Pivots & Sight-Line Calibration
// ---------------------------------------------------------------------------

/// Vanilla `ItemInHandRenderer` right-hand base translation (`HAND_OFFSET_X/Y/Z`).
pub(crate) const HAND_BASE: [f32; 3] = [0.56, -0.52, -0.72];

/// Places the Winchester's sight line (`X = 8.139`, `Y = 12.684` in `[0..16]` JSON texels,
/// midway between `alca_entalhe_base` and `massa_lamina`) dead-center on `[0, 0]` in view space
/// at `RIFLE_DISPLAY_SCALE = 0.60`.
pub(crate) const ADS_OFFSET: [f32; 3] = [-0.5277, 0.2944, 0.04];

/// Matches `display.firstperson_righthand` (`0.60` = crisp 1st-person viewmodel presence
/// while `thirdperson` `0.55` & `ground` `0.52` match authentic ~1.6-block rifle length).
pub(crate) const RIFLE_DISPLAY_TRANS: [f32; 3] = [-0.6 / 16.0, 0.8 / 16.0, -6.4 / 16.0];
pub(crate) const RIFLE_DISPLAY_SCALE: f32 = 0.60;

/// Exact muzzle crown (`boca_cano` `[7.731, 11.296, -16.0]..[8.547, 12.064, -15.52]`) in `[0..16]` texels.
pub(crate) const MUZZLE_TEXELS: [f32; 3] = [8.139, 11.68, -16.0];

/// Exact top receiver carrier / ejection port (`trava_dorsal` `[7.803, 12.032, 10.4]..[8.475, 12.224, 11.456]`
/// atop `culatra_principal` `Y = 9.824..12.128, Z = 7.52..13.824`) in `[0..16]` texels:
/// - `EJECTOR_START_TEXELS`: inside the brass carrier (`trava_dorsal`) so Stage 1 rises out of
///   the top slot clipped cleanly by the steel receiver walls;
/// - `EJECTOR_HANDOFF_TEXELS`: just clear of `trava_dorsal` (`Y = 12.78`), handed off at 0-pixel
///   offset to Stage 2 world-space tumbling physics.
pub(crate) const EJECTOR_START_TEXELS: [f32; 3] = [8.139, 11.42, 10.93];
pub(crate) const EJECTOR_HANDOFF_TEXELS: [f32; 3] = [8.42, 12.78, 11.05];

/// Mechanical rotation origins in `[0..16]` JSON texels (derived from `winchester_1873.bbmodel`).
pub(crate) const LEVER_PIVOT_TEX: [f32; 3] = [8.139, 8.912, 13.376];
pub(crate) const HAMMER_PIVOT_TEX: [f32; 3] = [8.139, 11.072, 15.488];
pub(crate) const TRIGGER_PIVOT_TEX: [f32; 3] = [8.139, 9.104, 14.768];
pub(crate) const GATE_PIVOT_TEX: [f32; 3] = [8.936, 10.592, 10.432];
pub(crate) const RECOIL_PIVOT_TEXELS: [f32; 3] = [8.14, 8.60, 22.00];

/// Hand grip anchors in `[0..16]` JSON texels:
/// - Right hand wraps around the angled walnut stock wrist (`coronha_105..116`, `Y=7.8..10.3, Z=16.5..18.0`);
/// - Left hand cradles the walnut forestock (`guarda_mao_traseiro`, `X=7.563..8.715, Y=9.856..11.744, Z=-0.64..7.52`).
pub(crate) const GRIP_WRIST_TEXELS: [f32; 3] = [9.55, 5.45, 16.50];
pub(crate) const FORESTOCK_TEXELS: [f32; 3] = [8.55, 8.05, 3.60];

/// First-person arm scale inside `[0..1]` rifle model space (`RIFLE_DISPLAY_SCALE = 0.60`):
/// `0.60 * 1.36 = 0.816` world blocks -> wide `4px` arm = `0.204` blocks (`~11%` larger than the
/// `0.18` walnut stock width so hands wrap naturally around the wrist & forestock without puffing up).
pub(crate) const ARM_GUN_SPACE_WIDTH: f32 = 1.36;
pub(crate) const ARM_GUN_SPACE_LENGTH: f32 = 1.54;

/// 3rd-person diagonal back-sling constants (`Hooks::render_player_post`).
pub(crate) const AVATAR_RENDER_SCALE: f32 = 0.9375;
pub(crate) const AVATAR_MODEL_OFFSET_Y: f32 = -1.501;
pub(crate) const THIRDPERSON_RIFLE_SCALE: f32 = 0.55;
pub(crate) const BACK_SLING_MODEL_SCALE: f32 = THIRDPERSON_RIFLE_SCALE / AVATAR_RENDER_SCALE;
pub(crate) const BACK_SLING_DIAG_DEG: f32 = -44.0;
pub(crate) const BACK_SLING_CENTER_TEX: [f32; 3] = [8.139, 8.50, 8.20];
pub(crate) const BACK_SLING_OFFSET_Y_TEX: f32 = -5.80;
pub(crate) const BACK_SLING_JACKET_Z_TEX: f32 = 3.10;
pub(crate) const BACK_SLING_ARMOR_Z_TEX: f32 = 3.85;
pub(crate) const BACK_SLING_REMOTE_Z_TEX: f32 = 3.55;

/// Horizon's 18 modern default player skins (`0..9` slim `3px`, `9..18` wide `4px`),
/// matching `horizon_client::player_skin::SKINS`.
pub(crate) const DEFAULT_SKINS: [(&str, bool); 18] = [
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
