//! Blackpowder Legacy (`v0.9.0`) — TAC-inspired Winchester Model 1873 for Horizon.
//!
//! Architecture (`v0.9.0` clean-room Rust port inspired by TAC `0.3.10.1`):
//! - **Multi-part animated weapon rendering (`models/special/winchester_*.json`):**
//!   `standalone_models` registers 6 sub-models (`body`, `lever`, `hammer`, `trigger`,
//!   `gate`, `cartridge`) baked into Horizon's model store and resolved as `ModelRef`s.
//!   In `render_hand`, when holding the Winchester in `Hand::Main`, vanilla's static item
//!   draw is cancelled (`Flow::Cancel`) and all 6 articulated parts + both 1st-person arms
//!   are posed in one unified `[0..1]` rifle space so sub-parts and hands never drift apart.
//! - **Real player skin + `slim`/`wide` 1st-person arms with outer sleeves:**
//!   Offline/immediate fallback resolves `default_skin(uuid)` (`18` modern vanilla skins,
//!   `0..9` slim `3x12x4`, `9..18` wide `4x12x4`) directly from `event.render.player` on
//!   frame 0. Online skins are resolved on the server via `ServerHost::profile_by_uuid`,
//!   decoding Base64 `"textures"` JSON into Horizon's `"skins/" + sha1_utf16le(hash)` sprite
//!   key + `"metadata"."model" == "slim"`, and synced once over `RifleSkinSync`.
//! - **TAC-style keyframe state machine & custom CC0 `.ogg` audio (`sounds.json`):**
//!   Full state controller covering `Draw`, `Idle`/`Walk`/`Sprint`/`ADS`, `Fire` + `Lever`
//!   cycle (cocking hammer, opening lever, cycling action, locking), `ReloadIntro`,
//!   per-cartridge `ReloadLoop` (brass `.44-40` round pushed through depressed loading gate),
//!   `ReloadEndNorm`, `ReloadEndEmpty` (full lever chambering on empty reload), `DryClick`,
//!   and `Inspect` (`I` key), synchronized with bundled CC0 `.ogg` firearm audio.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use horizon_mod_api::{
    BlockEntityDrawContext, CalculatePlayerTurnEvent, CameraPoseEvent, ClientHost,
    ClientResourcesReloadedEvent, ClientSetupEvent, ClientTickPreEvent, ComputeFovEvent,
    CubeDeformation, CubeListBuilder, EntityUuid, Flow, GuiDraw, GuiRenderPostEvent, Hand, Hooks,
    Host, HostSide, IPayloadContext, Identifier, ItemBarEvent, ItemKey, ItemPart, ItemStack,
    KeyConflictContext, KeyMappingPreEvent, KeyRegistration, LayerDefinition, LevelTickPostEvent,
    LivingRef, MeshDefinition, ModelBakingCompletedEvent, ModelPart, ModelRef, OnUseTickEvent,
    PartPose, ParticleKey, ParticleOptions, PayloadId, Registrar, RegistrationContext,
    ReleaseUsingEvent, RenderFramePreEvent, RenderHandEvent, ServerHost, ServerTickEvent,
    ServerTickPhase, ShouldCauseReequipAnimationEvent, SoundPlay, SoundSource,
    StandaloneModelForm, StandaloneModelsEvent, Text, ToClient, ToServer, UseDurationEvent, VarInt,
};

// ---------------------------------------------------------------------------
// Content identifiers & gameplay constants
// ---------------------------------------------------------------------------

const WINCHESTER_ID: &str = "blackpowder_legacy:winchester_1873";
const AMMO_ID: &str = "blackpowder_legacy:rifle_cartridge";

const MODEL_BODY_ID: &str = "blackpowder_legacy:special/winchester_body";
const MODEL_LEVER_ID: &str = "blackpowder_legacy:special/winchester_lever";
const MODEL_HAMMER_ID: &str = "blackpowder_legacy:special/winchester_hammer";
const MODEL_TRIGGER_ID: &str = "blackpowder_legacy:special/winchester_trigger";
const MODEL_GATE_ID: &str = "blackpowder_legacy:special/winchester_gate";
const MODEL_CARTRIDGE_ID: &str = "blackpowder_legacy:special/winchester_cartridge";

const SND_FIRE: &str = "blackpowder_legacy:item.winchester_fire";
const SND_THUMP: &str = "blackpowder_legacy:item.winchester_thump";
const SND_LEVER: &str = "blackpowder_legacy:item.winchester_lever";
const SND_COCK: &str = "blackpowder_legacy:item.winchester_cock";
const SND_RELOAD_INTRO: &str = "blackpowder_legacy:animation.winchester_reload_intro";
const SND_RELOAD_INSERT: &str = "blackpowder_legacy:animation.winchester_reload_insert";
const SND_RELOAD_END: &str = "blackpowder_legacy:animation.winchester_reload_end";
const SND_DRAW: &str = "blackpowder_legacy:animation.winchester_draw";
const SND_INSPECT: &str = "blackpowder_legacy:animation.winchester_inspect";

const KEY_CATEGORY: &str = "key.categories.blackpowder_legacy";
const KEY_RELOAD: &str = "key.blackpowder_legacy.reload";
const KEY_RELOAD_DEFAULT: &str = "key.keyboard.r";
const KEY_INSPECT: &str = "key.blackpowder_legacy.inspect";
const KEY_INSPECT_DEFAULT: &str = "key.keyboard.i";

const VANILLA_KEY_ATTACK: &str = "key.attack";
const VANILLA_KEY_USE: &str = "key.use";

/// Tubular magazine capacity (8 `.44-40` rounds; item damage `0` = full, `8` = empty).
const MAGAZINE_CAPACITY: i32 = 8;

/// Hitscan range in blocks.
const RANGE_BLOCKS: f64 = 96.0;

/// Ray step for solid-block line-of-sight checks.
const BLOCK_RAY_STEP: f64 = 0.25;

/// Extra hit radius around entity hitboxes, matching vanilla `ProjectileUtil`.
const ENTITY_HIT_INFLATE: f64 = 0.3;

/// Damage dealt by a `.44-40` round (14 HP = 7 hearts).
const SHOT_DAMAGE: f32 = 14.0;

/// Minimum ticks between shots: fire recoil (`3t`) + lever-action cycle (`11t`) = `14t` (`0.70s`).
const FIRE_COOLDOWN_TICKS: u64 = 14;
/// Delay after shot before the lever-action sound/throw starts (`3t` = `0.15s`).
const LEVER_DELAY_TICKS: u64 = 3;
/// Duration of the lever-action throw (`11t` = `0.55s`).
const LEVER_DURATION_TICKS: u64 = 11;

/// Ticks after reload start before the first cartridge seats into the loading gate (`8t` = `0.40s`).
const RELOAD_INTRO_TICKS: u64 = 8;
/// Ticks per single `.44-40` cartridge pushed into the King's side gate (`10t` = `0.50s`).
const RELOAD_SHELL_TICKS: u64 = 10;
/// Normal reload recovery ticks once magazine/ammo finishes (`8t` = `0.40s`).
const RELOAD_END_NORM_TICKS: u64 = 8;
/// Empty-reload finale ticks (tilts back + works lever to chamber round #1: `15t` = `0.75s`).
const RELOAD_END_EMPTY_TICKS: u64 = 15;
/// Equip/draw animation duration (`10t` = `0.50s`).
const DRAW_TICKS: f32 = 10.0;
/// Full weapon inspect duration (`68t` = `3.40s`).
const INSPECT_TICKS: f32 = 68.0;

/// Smooth ADS transition rate (~135 ms to settle, matching TAC `aimProgress`).
const ADS_RATE_PER_SEC: f32 = 7.8;

/// FOV multiplier at full ADS (~41° from a 70° base FOV).
const ADS_FOV_FACTOR: f32 = 0.58;

/// Mouse turn multiplier at full ADS.
const ADS_TURN_SENSITIVITY: f64 = 0.52;

/// Hip-fire angular spread in radians (~1.6°); ADS is pinpoint (`0.0`).
const HIP_SPREAD_RAD: f64 = 0.028;

// ---------------------------------------------------------------------------
// Exact model-space geometry (`winchester_1873.bbmodel` / `item/winchester_1873.json`)
// ---------------------------------------------------------------------------
//
// Horizon's `first_person.rs` (`item_pose` at `attack = 0`) places the right hand at
// `[0.56, -0.52 - equip * 0.6, -0.72]`. Our JSON's `display.firstperson_righthand`
// applies `translation = [0.0, 0.3 / 16.0, -4.75 / 16.0]`, `scale = 0.45`, followed by
// `translate([-0.5, -0.5, -0.5])` so `[0..1]` model units map to `[0..16]` JSON texels.
//
// Sight axis inside `[0..16]` JSON texels:
//   - Rear buckhorn notch (`alca_entalhe_base` + `alca_parede_esq/dir`): X = 8.139, Y = 11.46
//   - Front blade tip (`massa_lamina`, Y = 11.248..11.616):             X = 8.139, Y = 11.55
// At `[0.56, -0.52, -0.72]`, the sight line sits at `[+0.5639, -0.3695, ...]`, so
// `ADS_OFFSET = [-0.5639, +0.3695, +0.05]` locks the front blade centered in the rear
// notch at `[0.0, 0.0]` eye height.

const HAND_BASE: [f32; 3] = [0.56, -0.52, -0.72];
const RIFLE_DISPLAY_TRANS: [f32; 3] = [0.0, 0.3 / 16.0, -4.75 / 16.0];
const RIFLE_DISPLAY_SCALE: f32 = 0.45;
const ADS_OFFSET: [f32; 3] = [-0.5639, 0.3695, 0.05];

/// Sub-model pivots in `[0..16]` JSON texel coordinates (`BB -> JM`: `8 + x`, `y`, `8 + z`).
const LEVER_PIVOT_TEX: [f32; 3] = [8.139, 8.912, 13.376];
const HAMMER_PIVOT_TEX: [f32; 3] = [8.139, 11.072, 15.488];
const TRIGGER_PIVOT_TEX: [f32; 3] = [8.139, 9.104, 14.768];
const GATE_PIVOT_TEX: [f32; 3] = [8.936, 10.592, 10.432];

/// TAC `m870_static.gltf` arm scale inside the `0.45` weapon pose: `[1.0, 1.55, 1.0]`
/// (`[0.45, 0.70, 0.45]` world), giving both arms identical `1:1` texel cross-section and reach.
const ARM_GUN_SPACE_WIDTH: f32 = 1.0;
const ARM_GUN_SPACE_LENGTH: f32 = 1.55;

// ---------------------------------------------------------------------------
// Modern 18-skin default table (`crates/horizon-client-render/src/skins.rs`)
// ---------------------------------------------------------------------------

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
// Payloads
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

#[derive(horizon_mod_api::PayloadCodec)]
struct RifleInput {
    action: VarInt,
}

#[derive(horizon_mod_api::PayloadCodec)]
struct RifleSync {
    kind: VarInt,
    rounds: VarInt,
}

#[derive(horizon_mod_api::PayloadCodec)]
struct RifleSkinSync {
    sprite: String,
    fallback: String,
    slim: VarInt,
}

// ---------------------------------------------------------------------------
// Static handles & runtime state
// ---------------------------------------------------------------------------

struct Handles {
    winchester: ItemKey,
    ammo: ItemKey,
    input_channel: PayloadId<RifleInput, ToServer>,
    sync_channel: PayloadId<RifleSync, ToClient>,
    skin_channel: PayloadId<RifleSkinSync, ToClient>,
    smoke_particle: ParticleKey,
    crit_particle: ParticleKey,
    flame_particle: ParticleKey,
}

#[derive(Clone, Copy)]
struct RifleModels {
    body: ModelRef,
    lever: ModelRef,
    hammer: ModelRef,
    trigger: ModelRef,
    gate: ModelRef,
    cartridge: ModelRef,
}

struct PlayerArms {
    wide_right: ModelPart,
    wide_left: ModelPart,
    slim_right: ModelPart,
    slim_left: ModelPart,
}

#[derive(Clone)]
struct ResolvedSkin {
    primary: Identifier,
    fallback: Identifier,
    slim: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AnimMode {
    None,
    Draw,
    FireCycle { cock_sound_played: bool, lever_sound_played: bool, keep_uncocked: bool },
    DryClick,
    ReloadIntro { from_empty: bool },
    ReloadLoop { from_empty: bool },
    ReloadEndNorm,
    ReloadEndEmpty { lever_sound_played: bool, cock_sound_played: bool },
    Inspect,
}

struct ClientState {
    aim_down: bool,
    aim_progress: f32,
    holding_rifle: bool,
    was_holding_rifle: bool,
    last_slot: u8,
    known_rounds: i32,
    hammer_uncocked: bool,
    sprinting: bool,
    walk_phase: f32,
    walk_speed: f32,
    idle_time: f32,
    recoil: f32,
    camera_pitch_kick: f32,
    camera_yaw_kick: f32,
    muzzle_flash: f32,
    mode: AnimMode,
    mode_ticks: f32,
    skin_requested_uuid: Option<u128>,
    cached_default_uuid: Option<(u128, ResolvedSkin)>,
    server_skin: Option<(u128, ResolvedSkin)>,
}

impl ClientState {
    const fn new() -> Self {
        Self {
            aim_down: false,
            aim_progress: 0.0,
            holding_rifle: false,
            was_holding_rifle: false,
            last_slot: 0,
            known_rounds: MAGAZINE_CAPACITY,
            hammer_uncocked: false,
            sprinting: false,
            walk_phase: 0.0,
            walk_speed: 0.0,
            idle_time: 0.0,
            recoil: 0.0,
            camera_pitch_kick: 0.0,
            camera_yaw_kick: 0.0,
            muzzle_flash: 0.0,
            mode: AnimMode::None,
            mode_ticks: 0.0,
            skin_requested_uuid: None,
            cached_default_uuid: None,
            server_skin: None,
        }
    }

    fn is_reloading(&self) -> bool {
        matches!(
            self.mode,
            AnimMode::ReloadIntro { .. }
                | AnimMode::ReloadLoop { .. }
                | AnimMode::ReloadEndNorm
                | AnimMode::ReloadEndEmpty { .. }
        )
    }

    fn blocks_ads(&self) -> bool {
        matches!(
            self.mode,
            AnimMode::Draw
                | AnimMode::ReloadIntro { .. }
                | AnimMode::ReloadLoop { .. }
                | AnimMode::ReloadEndNorm
                | AnimMode::ReloadEndEmpty { .. }
                | AnimMode::Inspect
        )
    }
}

#[derive(Clone, Copy)]
struct ReloadTracker {
    slot: u8,
    next_insert_tick: u64,
    from_empty: bool,
    inserted_any: bool,
    finishing_until: Option<(u64, bool)>,
}

#[derive(Default)]
struct ServerState {
    tick: u64,
    next_fire_tick: HashMap<u128, u64>,
    reloading: HashMap<u128, ReloadTracker>,
}

static HANDLES: OnceLock<Handles> = OnceLock::new();
static PLAYER_ARMS: OnceLock<PlayerArms> = OnceLock::new();
static RIFLE_MODELS: Mutex<Option<RifleModels>> = Mutex::new(None);
static CLIENT: Mutex<ClientState> = Mutex::new(ClientState::new());
static SERVER: Mutex<ServerState> = Mutex::new(ServerState {
    tick: 0,
    next_fire_tick: HashMap::new(),
    reloading: HashMap::new(),
});

fn handles() -> &'static Handles {
    HANDLES.get().expect("Handles initialized during register()")
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

/// Extracts `(skins/<sha1>, is_slim)` from Mojang's Base64 `"textures"` property JSON.
fn parse_mojang_skin_property(b64_value: &str) -> Option<(String, bool)> {
    let raw = base64_decode(b64_value)?;
    let json = core::str::from_utf8(&raw).ok()?;
    let skin_idx = json.find("\"SKIN\"")?;
    let skin_tail = &json[skin_idx..];
    // Bound to the SKIN object (before any sibling `"CAPE"` entry).
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

fn build_player_arms() -> PlayerArms {
    let build_one = |origin_x: f32, width: f32, skin_uv: (i32, i32), sleeve_uv: (i32, i32)| {
        let mut mesh = MeshDefinition::new();
        mesh.root().add_or_replace_child(
            "arm",
            CubeListBuilder::create()
                .tex_offs(skin_uv.0, skin_uv.1)
                .add_box([origin_x, -2.0, -2.0], [width, 12.0, 4.0])
                .tex_offs(sleeve_uv.0, sleeve_uv.1)
                .add_box_with(
                    [origin_x, -2.0, -2.0],
                    [width, 12.0, 4.0],
                    CubeDeformation::new(0.25),
                ),
            PartPose::ZERO,
        );
        LayerDefinition::create(mesh, 64, 64)
            .bake_root()
            .child("arm")
            .expect("arm child exists")
    };

    PlayerArms {
        wide_right: build_one(-3.0, 4.0, (40, 16), (40, 32)),
        wide_left: build_one(-1.0, 4.0, (32, 48), (48, 48)),
        slim_right: build_one(-2.0, 3.0, (40, 16), (40, 32)),
        slim_left: build_one(-1.0, 3.0, (32, 48), (48, 48)),
    }
}

fn resolve_rifle_models(host: &dyn ClientHost) -> Option<RifleModels> {
    let assets = host.render_assets();
    Some(RifleModels {
        body: assets.model(&Identifier::parse(MODEL_BODY_ID).ok()?).ok()?,
        lever: assets.model(&Identifier::parse(MODEL_LEVER_ID).ok()?).ok()?,
        hammer: assets.model(&Identifier::parse(MODEL_HAMMER_ID).ok()?).ok()?,
        trigger: assets.model(&Identifier::parse(MODEL_TRIGGER_ID).ok()?).ok()?,
        gate: assets.model(&Identifier::parse(MODEL_GATE_ID).ok()?).ok()?,
        cartridge: assets.model(&Identifier::parse(MODEL_CARTRIDGE_ID).ok()?).ok()?,
    })
}

fn refresh_rifle_models(host: &dyn ClientHost) {
    let resolved = resolve_rifle_models(host);
    *RIFLE_MODELS.lock().unwrap() = resolved;
}

// ---------------------------------------------------------------------------
// Local client sound helper (`sounds.json` CC0 `.ogg` pack)
// ---------------------------------------------------------------------------

fn play_gun_sound(host: &dyn ClientHost, event_id: &str, volume: f32, pitch: f32) {
    let Ok(identifier) = Identifier::parse(event_id) else {
        return;
    };
    let sound = SoundPlay {
        identifier,
        source: SoundSource::Players,
        volume,
        pitch,
        position: [0.0, 0.0, 0.0],
        looping: false,
        delay: 0,
        attenuation: false,
        relative: true,
    };
    let _ = host.play_local_sound(&sound, 0);
}

// ---------------------------------------------------------------------------
// Item Part — Winchester 1873
// ---------------------------------------------------------------------------

struct WinchesterItem;

impl ItemPart for WinchesterItem {
    /// Suppresses vanilla equip-bob when magazine damage changes on shot/reload.
    fn should_cause_reequip_animation(&self, event: &ShouldCauseReequipAnimationEvent) -> bool {
        event.old_stack.item() != event.new_stack.item()
    }

    fn use_duration(&self, _: &dyn Host, _: &UseDurationEvent<'_>) -> Option<i32> {
        Some(72_000)
    }

    fn on_use_tick(&self, _: &mut dyn Host, _: &mut OnUseTickEvent<'_>) {}

    fn release_using(&self, _: &mut dyn Host, _: &mut ReleaseUsingEvent<'_>) {}

    fn bar_visible(&self, stack: &ItemStack) -> Option<bool> {
        Some(rounds_in_stack(stack) < MAGAZINE_CAPACITY)
    }

    fn bar_width(&self, stack: &ItemStack) -> Option<i32> {
        let rounds = rounds_in_stack(stack);
        Some(((rounds * 13 + MAGAZINE_CAPACITY / 2) / MAGAZINE_CAPACITY).clamp(0, 13))
    }

    fn bar_color(&self, _: &ItemBarEvent<'_>) -> Option<u32> {
        Some(0xD8_A6_48)
    }
}

fn rounds_in_stack(stack: &ItemStack) -> i32 {
    (MAGAZINE_CAPACITY - stack.damage()).clamp(0, MAGAZINE_CAPACITY)
}

fn set_rounds_in_stack(stack: &mut ItemStack, rounds: i32) {
    let clamped = rounds.clamp(0, MAGAZINE_CAPACITY);
    stack.set_damage((MAGAZINE_CAPACITY - clamped).max(0));
}

fn is_winchester(stack: &ItemStack) -> bool {
    !stack.is_empty() && stack.item() == handles().winchester.id()
}

// ---------------------------------------------------------------------------
// Entry Point & Hooks
// ---------------------------------------------------------------------------

pub struct Mod;

#[horizon_mod_api::mod_entry]
pub fn init() -> Mod {
    Mod
}

impl Hooks for Mod {
    fn standalone_models(&self, _: &RegistrationContext, event: &mut StandaloneModelsEvent<'_>) {
        for model_id in [
            MODEL_BODY_ID,
            MODEL_LEVER_ID,
            MODEL_HAMMER_ID,
            MODEL_TRIGGER_ID,
            MODEL_GATE_ID,
            MODEL_CARTRIDGE_ID,
        ] {
            if let Ok(id) = Identifier::parse(model_id) {
                let _ = event.register(id, StandaloneModelForm::Block);
            }
        }
    }

    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let winchester = r
            .items()
            .expect("registries capability")
            .register_Damageable(WINCHESTER_ID, MAGAZINE_CAPACITY)
            .unwrap_or_else(|e| panic!("failed to register {WINCHESTER_ID}: {e}"));

        let ammo = r
            .items()
            .expect("registries capability")
            .register_Simple(AMMO_ID)
            .unwrap_or_else(|e| panic!("failed to register {AMMO_ID}: {e}"));

        r.item(winchester, WinchesterItem);

        r.key_mapping(
            KeyRegistration::new(KEY_RELOAD, KEY_RELOAD_DEFAULT, KEY_CATEGORY)
                .conflict_context(KeyConflictContext::InGame),
        );
        r.key_mapping(
            KeyRegistration::new(KEY_INSPECT, KEY_INSPECT_DEFAULT, KEY_CATEGORY)
                .conflict_context(KeyConflictContext::InGame),
        );

        let smoke_particle = r.particle_type("minecraft:large_smoke").unwrap();
        let crit_particle = r.particle_type("minecraft:crit").unwrap();
        let flame_particle = r.particle_type("minecraft:flame").unwrap();

        let mut net = r.registrar("1");
        let input_channel = net
            .play_to_server::<RifleInput>(
                Identifier::parse("blackpowder_legacy:rifle_input").unwrap(),
                handle_rifle_input,
            )
            .unwrap();
        let sync_channel = net
            .play_to_client::<RifleSync>(
                Identifier::parse("blackpowder_legacy:rifle_sync").unwrap(),
                handle_rifle_sync,
            )
            .unwrap();
        let skin_channel = net
            .play_to_client::<RifleSkinSync>(
                Identifier::parse("blackpowder_legacy:rifle_skin").unwrap(),
                handle_rifle_skin_sync,
            )
            .unwrap();

        let _ = HANDLES.set(Handles {
            winchester,
            ammo,
            input_channel,
            sync_channel,
            skin_channel,
            smoke_particle,
            crit_particle,
            flame_particle,
        });
    }

    fn model_baking_completed(&self, host: &mut dyn ClientHost, _: &mut ModelBakingCompletedEvent) {
        refresh_rifle_models(host);
    }

    fn client_setup(&self, host: &mut dyn ClientHost, _: &mut ClientSetupEvent) {
        let _ = PLAYER_ARMS.set(build_player_arms());
        refresh_rifle_models(host);
    }

    fn client_resources_reloaded(
        &self,
        host: &mut dyn ClientHost,
        _: &mut ClientResourcesReloadedEvent,
    ) {
        refresh_rifle_models(host);
    }

    fn key_mapping_pre(&self, host: &mut dyn ClientHost, event: &mut KeyMappingPreEvent<'_>) -> Flow {
        if event.name != VANILLA_KEY_ATTACK {
            return Flow::Continue;
        }
        let Some(player) = host.local_player() else {
            return Flow::Continue;
        };
        let stack = player.as_living().main_hand_item(host);
        if !is_winchester(&stack) {
            return Flow::Continue;
        }

        if event.pressed {
            let can_request_fire = {
                let mut st = CLIENT.lock().unwrap();
                if st.mode == AnimMode::Draw
                    || matches!(st.mode, AnimMode::FireCycle { .. })
                    || matches!(
                        st.mode,
                        AnimMode::ReloadEndNorm | AnimMode::ReloadEndEmpty { .. }
                    )
                {
                    false
                } else {
                    if st.mode == AnimMode::Inspect {
                        st.mode = AnimMode::None;
                        st.mode_ticks = 0.0;
                    }
                    true
                }
            };
            if can_request_fire {
                let _ = host.send_to_server(
                    handles().input_channel,
                    &RifleInput {
                        action: VarInt(ACTION_FIRE),
                    },
                );
            }
        }
        Flow::Cancel
    }

    fn client_tick_pre(&self, host: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
        let Some(player) = host.local_player() else {
            let mut st = CLIENT.lock().unwrap();
            st.aim_down = false;
            st.holding_rifle = false;
            st.was_holding_rifle = false;
            st.mode = AnimMode::None;
            st.skin_requested_uuid = None;
            return;
        };

        let uuid = player.as_entity().uuid(host);
        let slot = player.selected_slot(host);
        let stack = player.as_living().main_hand_item(host);
        let holding = is_winchester(&stack);
        let rounds = if holding { rounds_in_stack(&stack) } else { 0 };
        let screen_open = host.has_open_screen();
        let use_down = !screen_open && holding && host.is_key_down(VANILLA_KEY_USE);

        let mut reload_clicks = 0u32;
        while host.consume_key_click(KEY_RELOAD) {
            reload_clicks += 1;
        }
        let mut inspect_clicks = 0u32;
        while host.consume_key_click(KEY_INSPECT) {
            inspect_clicks += 1;
        }

        let sprinting = player.as_entity().sprinting(host).unwrap_or(false);
        let vel = player
            .as_entity()
            .velocity(host)
            .unwrap_or([0.0, 0.0, 0.0]);
        let horiz_speed = ((vel[0] * vel[0] + vel[2] * vel[2]).sqrt() as f32).min(0.45);
        let on_ground = player.as_entity().on_ground(host).unwrap_or(true);

        let mut send_reload = false;
        let mut send_cancel_reload = false;
        let mut send_skin_req = false;
        let mut play_sound: Option<(&'static str, f32, f32)> = None;

        {
            let mut st = CLIENT.lock().unwrap();
            let newly_equipped = holding && (!st.was_holding_rifle || st.last_slot != slot);
            st.was_holding_rifle = holding;
            st.last_slot = slot;
            st.holding_rifle = holding;
            st.sprinting = sprinting;
            st.known_rounds = rounds;

            if holding && st.skin_requested_uuid != Some(uuid) {
                st.skin_requested_uuid = Some(uuid);
                send_skin_req = true;
            }

            if !holding {
                if st.is_reloading() {
                    send_cancel_reload = true;
                }
                st.aim_down = false;
                st.mode = AnimMode::None;
                st.mode_ticks = 0.0;
            } else {
                if newly_equipped {
                    st.mode = AnimMode::Draw;
                    st.mode_ticks = 0.0;
                    st.hammer_uncocked = rounds == 0;
                    play_sound = Some((SND_DRAW, 0.85, 1.0));
                }

                // Sprint cancels inspect or active reload cleanly.
                if sprinting && !use_down {
                    if st.is_reloading() {
                        st.mode = AnimMode::None;
                        st.mode_ticks = 0.0;
                        send_cancel_reload = true;
                    } else if st.mode == AnimMode::Inspect {
                        st.mode = AnimMode::None;
                        st.mode_ticks = 0.0;
                    }
                }

                // Aiming down sights cancels inspect immediately.
                if use_down && st.mode == AnimMode::Inspect {
                    st.mode = AnimMode::None;
                    st.mode_ticks = 0.0;
                }

                st.aim_down = use_down && !st.blocks_ads() && !sprinting;

                if !screen_open && reload_clicks > 0 && !st.is_reloading() && st.mode != AnimMode::Draw {
                    send_reload = true;
                } else if !screen_open
                    && inspect_clicks > 0
                    && st.mode == AnimMode::None
                    && !st.aim_down
                    && !sprinting
                {
                    st.mode = AnimMode::Inspect;
                    st.mode_ticks = 0.0;
                    play_sound = Some((SND_INSPECT, 0.90, 1.0));
                }
            }

            // Walk / idle bobbing phase update (20 Hz tick).
            let target_walk = if holding && on_ground && horiz_speed > 0.015 {
                (horiz_speed / 0.22).clamp(0.0, 1.45)
            } else {
                0.0
            };
            st.walk_speed += (target_walk - st.walk_speed) * 0.35;
            st.walk_phase = (st.walk_phase
                + st.walk_speed * if sprinting { 0.72 } else { 0.52 })
                % core::f32::consts::TAU;
        }

        if let Some((snd, vol, pitch)) = play_sound {
            play_gun_sound(host, snd, vol, pitch);
        }
        if send_skin_req {
            let _ = host.send_to_server(
                handles().input_channel,
                &RifleInput {
                    action: VarInt(ACTION_SKIN_REQUEST),
                },
            );
        }
        if send_cancel_reload {
            let _ = host.send_to_server(
                handles().input_channel,
                &RifleInput {
                    action: VarInt(ACTION_RELOAD_CANCEL),
                },
            );
        }
        if send_reload {
            let _ = host.send_to_server(
                handles().input_channel,
                &RifleInput {
                    action: VarInt(ACTION_RELOAD),
                },
            );
        }
    }

    fn server_tick(&self, _: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
        if event.phase != ServerTickPhase::Post {
            return;
        }
        let mut srv = SERVER.lock().unwrap();
        srv.tick = srv.tick.wrapping_add(1);
    }

    fn level_tick_post(&self, host: HostSide<'_>, _: &mut LevelTickPostEvent) {
        let HostSide::Server(host) = host else {
            return;
        };
        if host.dimension() != "minecraft:overworld" {
            return;
        }

        let (now, active_reloads) = {
            let srv = SERVER.lock().unwrap();
            let now = srv.tick;
            let reloads: Vec<(u128, ReloadTracker)> =
                srv.reloading.iter().map(|(&k, &v)| (k, v)).collect();
            (now, reloads)
        };

        for (uuid, tracker) in active_reloads {
            step_player_reload(host, uuid, tracker, now);
        }
    }

    fn render_frame_pre(&self, host: &mut dyn ClientHost, event: &mut RenderFramePreEvent) {
        let dt = event.frame_dt_seconds.clamp(0.0, 0.1);
        let dt_ticks = dt * 20.0;

        let mut triggered_sounds: Vec<(&'static str, f32, f32)> = Vec::new();
        {
            let mut st = CLIENT.lock().unwrap();
            let target_ads = if st.holding_rifle && st.aim_down && !st.blocks_ads() {
                1.0
            } else {
                0.0
            };
            let step = ADS_RATE_PER_SEC * dt;
            if st.aim_progress < target_ads {
                st.aim_progress = (st.aim_progress + step).min(target_ads);
            } else if st.aim_progress > target_ads {
                st.aim_progress = (st.aim_progress - step).max(target_ads);
            }

            st.idle_time = (st.idle_time + dt) % 600.0;
            st.recoil = (st.recoil - dt * 5.2).max(0.0);
            st.muzzle_flash = (st.muzzle_flash - dt * 16.0).max(0.0);
            let cam_decay = (-dt * 12.0).exp();
            st.camera_pitch_kick *= cam_decay;
            st.camera_yaw_kick *= cam_decay;

            st.mode_ticks += dt_ticks;
            match st.mode {
                AnimMode::None => {}
                AnimMode::Draw => {
                    if st.mode_ticks >= DRAW_TICKS {
                        st.mode = AnimMode::None;
                        st.mode_ticks = 0.0;
                    }
                }
                AnimMode::FireCycle {
                    mut cock_sound_played,
                    mut lever_sound_played,
                    keep_uncocked,
                } => {
                    if keep_uncocked {
                        // Last round fired: hammer stays down on empty chamber, no lever throw.
                        st.hammer_uncocked = true;
                        if st.mode_ticks >= 5.0 {
                            st.mode = AnimMode::None;
                            st.mode_ticks = 0.0;
                        } else {
                            st.mode = AnimMode::FireCycle {
                                cock_sound_played: true,
                                lever_sound_played: true,
                                keep_uncocked: true,
                            };
                        }
                    } else {
                        let total = (LEVER_DELAY_TICKS + LEVER_DURATION_TICKS) as f32;
                        if !lever_sound_played && st.mode_ticks >= LEVER_DELAY_TICKS as f32 {
                            lever_sound_played = true;
                            triggered_sounds.push((SND_LEVER, 0.85, 1.0));
                        }
                        if !cock_sound_played && st.mode_ticks >= (LEVER_DELAY_TICKS as f32 + 5.2) {
                            cock_sound_played = true;
                            triggered_sounds.push((SND_COCK, 0.72, 1.04));
                        }
                        if st.mode_ticks >= total {
                            st.hammer_uncocked = false;
                            st.mode = AnimMode::None;
                            st.mode_ticks = 0.0;
                        } else {
                            st.mode = AnimMode::FireCycle {
                                cock_sound_played,
                                lever_sound_played,
                                keep_uncocked,
                            };
                        }
                    }
                }
                AnimMode::DryClick => {
                    st.hammer_uncocked = true;
                    if st.mode_ticks >= 5.0 {
                        st.mode = AnimMode::None;
                        st.mode_ticks = 0.0;
                    }
                }
                AnimMode::ReloadIntro { .. } | AnimMode::ReloadLoop { .. } => {
                    // Safety timeout in case a server sync packet was dropped across level change.
                    if st.mode_ticks > 40.0 {
                        st.mode = AnimMode::None;
                        st.mode_ticks = 0.0;
                    }
                }
                AnimMode::ReloadEndNorm => {
                    if st.mode_ticks >= RELOAD_END_NORM_TICKS as f32 {
                        st.mode = AnimMode::None;
                        st.mode_ticks = 0.0;
                    }
                }
                AnimMode::ReloadEndEmpty {
                    mut lever_sound_played,
                    mut cock_sound_played,
                } => {
                    if !lever_sound_played && st.mode_ticks >= 3.0 {
                        lever_sound_played = true;
                        triggered_sounds.push((SND_LEVER, 0.90, 0.98));
                    }
                    if !cock_sound_played && st.mode_ticks >= 8.2 {
                        cock_sound_played = true;
                        st.hammer_uncocked = false;
                        triggered_sounds.push((SND_COCK, 0.78, 1.0));
                    }
                    if st.mode_ticks >= RELOAD_END_EMPTY_TICKS as f32 {
                        st.hammer_uncocked = false;
                        st.mode = AnimMode::None;
                        st.mode_ticks = 0.0;
                    } else {
                        st.mode = AnimMode::ReloadEndEmpty {
                            lever_sound_played,
                            cock_sound_played,
                        };
                    }
                }
                AnimMode::Inspect => {
                    if st.mode_ticks >= INSPECT_TICKS {
                        st.mode = AnimMode::None;
                        st.mode_ticks = 0.0;
                    }
                }
            }
        }

        for (snd, vol, pitch) in triggered_sounds {
            play_gun_sound(host, snd, vol, pitch);
        }
    }

    fn compute_fov(&self, _: &dyn ClientHost, event: &mut ComputeFovEvent) {
        let st = CLIENT.lock().unwrap();
        let t = smoothstep(st.aim_progress);
        if t <= 0.0001 {
            return;
        }
        let factor = 1.0 + (ADS_FOV_FACTOR - 1.0) * t;
        event.fov *= factor;
    }

    fn calculate_player_turn(&self, _: &dyn ClientHost, event: &mut CalculatePlayerTurnEvent) {
        let st = CLIENT.lock().unwrap();
        let t = smoothstep(st.aim_progress) as f64;
        if t <= 0.0001 {
            return;
        }
        let mult = 1.0 + (ADS_TURN_SENSITIVITY - 1.0) * t;
        event.y_rot *= mult;
        event.x_rot *= mult;
    }

    fn camera_pose(&self, _: &dyn ClientHost, event: &mut CameraPoseEvent<'_>) {
        let st = CLIENT.lock().unwrap();
        if st.camera_pitch_kick.abs() > 0.001 || st.camera_yaw_kick.abs() > 0.001 {
            event.angles.pitch -= st.camera_pitch_kick;
            event.angles.yaw += st.camera_yaw_kick;
        }
    }

    fn render_hand(&self, _: &dyn ClientHost, event: &mut RenderHandEvent<'_, '_>) -> Flow {
        let (
            holding,
            aim,
            recoil,
            flash,
            sprinting,
            walk_phase,
            walk_speed,
            idle_time,
            hammer_uncocked,
            mode,
            mode_ticks,
            skin,
        ) = {
            let mut st = CLIENT.lock().unwrap();
            let uuid = event.render.player;
            let skin = resolve_client_skin(&mut st, uuid);
            (
                st.holding_rifle,
                smoothstep(st.aim_progress),
                st.recoil,
                st.muzzle_flash,
                st.sprinting,
                st.walk_phase,
                st.walk_speed,
                st.idle_time,
                st.hammer_uncocked,
                st.mode,
                st.mode_ticks,
                skin,
            )
        };

        if !holding {
            return Flow::Continue;
        }
        if event.hand == Hand::Off {
            // Suppress off-hand item while wielding the two-handed lever-action rifle.
            return Flow::Cancel;
        }
        if !is_winchester(&event.render.item) {
            return Flow::Continue;
        }

        let Some(models) = *RIFLE_MODELS.lock().unwrap() else {
            return Flow::Continue;
        };

        let pose = evaluate_weapon_pose(
            aim,
            recoil,
            flash,
            sprinting,
            walk_phase,
            walk_speed,
            idle_time,
            hammer_uncocked,
            mode,
            mode_ticks,
        );

        let equip = event.render.equip_progress.clamp(0.0, 1.0) * (1.0 - aim);
        let draw = &mut event.draw;
        draw.push();

        // 1. Animated weapon root pose in camera view space.
        draw.translate(pose.gun_trans);
        draw.rotate_about([1.0, 0.0, 0.0], pose.gun_rot[0], [0.0; 3]);
        draw.rotate_about([0.0, 1.0, 0.0], pose.gun_rot[1], [0.0; 3]);
        draw.rotate_about([0.0, 0.0, 1.0], pose.gun_rot[2], [0.0; 3]);
        draw.scale([pose.gun_scale; 3]);

        // 2. Vanilla right-hand anchor (`first_person.rs` `item_pose` at `attack = 0`) +
        //    `display.firstperson_righthand` (`[0, 0.3, -4.75] / 16`, `scale = 0.45`) +
        //    `[-0.5, -0.5, -0.5]` unit-block centering so `[0..1]` equals `[0..16]` JSON texels.
        draw.translate([HAND_BASE[0], HAND_BASE[1] - equip * 0.6, HAND_BASE[2]]);
        draw.translate(RIFLE_DISPLAY_TRANS);
        draw.scale([RIFLE_DISPLAY_SCALE; 3]);
        draw.translate([-0.5, -0.5, -0.5]);

        // 3. Draw receiver/barrel/stock body (`winchester_body.json`).
        if draw.model(models.body, 0xFF_FF_FF_FF).is_err() {
            draw.pop();
            return Flow::Continue;
        }

        // 4. Draw articulated lever (`winchester_lever.json`).
        draw.push();
        draw.rotate_about([1.0, 0.0, 0.0], pose.lever_deg, tex_to_unit(LEVER_PIVOT_TEX));
        let _ = draw.model(models.lever, 0xFF_FF_FF_FF);
        draw.pop();

        // 5. Draw articulated hammer (`winchester_hammer.json`).
        draw.push();
        draw.rotate_about(
            [1.0, 0.0, 0.0],
            pose.hammer_deg,
            tex_to_unit(HAMMER_PIVOT_TEX),
        );
        let _ = draw.model(models.hammer, 0xFF_FF_FF_FF);
        draw.pop();

        // 6. Draw articulated trigger (`winchester_trigger.json`).
        draw.push();
        draw.rotate_about(
            [1.0, 0.0, 0.0],
            pose.trigger_deg,
            tex_to_unit(TRIGGER_PIVOT_TEX),
        );
        let _ = draw.model(models.trigger, 0xFF_FF_FF_FF);
        draw.pop();

        // 7. Draw King's side loading gate (`winchester_gate.json`).
        draw.push();
        draw.rotate_about([0.0, 1.0, 0.0], pose.gate_deg, tex_to_unit(GATE_PIVOT_TEX));
        let _ = draw.model(models.gate, 0xFF_FF_FF_FF);
        draw.pop();

        // 8. Draw `.44-40` brass cartridge during per-shell gate insertion.
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

        // 9. Draw both 1st-person player arms (`wide` 4px or `slim` 3px + 3D sleeves)
        //    directly inside the gun's `[0..1]` frame with identical proportions.
        if let Some(arms) = PLAYER_ARMS.get() {
            let (right_part, left_part) = if skin.slim {
                (&arms.slim_right, &arms.slim_left)
            } else {
                (&arms.wide_right, &arms.wide_left)
            };
            draw_gun_space_arm(
                draw,
                right_part,
                &skin,
                true,
                pose.right_hand_tex,
                pose.right_hand_rot,
            );
            draw_gun_space_arm(
                draw,
                left_part,
                &skin,
                false,
                pose.left_hand_tex,
                pose.left_hand_rot,
            );
        }

        draw.pop();
        Flow::Cancel
    }

    fn gui_render_post(&self, _: &dyn ClientHost, event: &mut GuiRenderPostEvent<'_>) {
        let (holding, aim, reloading, rounds) = {
            let st = CLIENT.lock().unwrap();
            (
                st.holding_rifle,
                smoothstep(st.aim_progress),
                st.is_reloading(),
                st.known_rounds,
            )
        };
        if !holding {
            return;
        }

        let w = event.width;
        let h = event.height;
        let cx = w / 2;
        let cy = h / 2;

        // Subtle focus vignette when aiming down iron sights (center stays crystal clear).
        if aim > 0.01 {
            draw_ads_vignette(&mut event.gui, w, h, aim);
        }

        // Dynamic hip-fire crosshair that fades cleanly out as iron sights align.
        let hip_alpha = ((1.0 - aim * 1.45).clamp(0.0, 1.0) * 185.0) as u32;
        if hip_alpha > 4 && !reloading {
            let color = (hip_alpha << 24) | 0x00F4_EFE6;
            let shadow = ((hip_alpha / 2) << 24) | 0x0012_100E;
            let gap = 5;
            let tick = 3;
            event.gui.fill([cx - gap - tick, cy], [cx - gap, cy + 1], color);
            event.gui.fill([cx + gap + 1, cy], [cx + gap + 1 + tick, cy + 1], color);
            event.gui.fill([cx, cy - gap - tick], [cx + 1, cy - gap], color);
            event.gui.fill([cx, cy + gap + 1], [cx + 1, cy + gap + 1 + tick], color);
            event.gui.fill([cx - gap - tick, cy + 1], [cx - gap, cy + 2], shadow);
            event.gui.fill([cx + gap + 1, cy + 1], [cx + gap + 1 + tick, cy + 2], shadow);
        }

        // Compact western brass ammo counter above right hotbar (`8 / 8`).
        draw_ammo_hud(&mut event.gui, w, h, rounds, reloading);
    }
}

// ---------------------------------------------------------------------------
// Keyframe Weapon & Dual-Arm Pose Evaluator
// ---------------------------------------------------------------------------

struct WeaponPose {
    gun_trans: [f32; 3],
    gun_rot: [f32; 3],
    gun_scale: f32,
    lever_deg: f32,
    hammer_deg: f32,
    trigger_deg: f32,
    gate_deg: f32,
    show_cartridge: bool,
    cartridge_tex: [f32; 3],
    cartridge_rot: [f32; 3],
    right_hand_tex: [f32; 3],
    right_hand_rot: [f32; 3],
    left_hand_tex: [f32; 3],
    left_hand_rot: [f32; 3],
}

#[allow(clippy::too_many_arguments)]
fn evaluate_weapon_pose(
    aim: f32,
    recoil: f32,
    flash: f32,
    sprinting: bool,
    walk_phase: f32,
    walk_speed: f32,
    idle_time: f32,
    hammer_uncocked: bool,
    mode: AnimMode,
    mode_ticks: f32,
) -> WeaponPose {
    let hip = 1.0 - aim;

    // Base root pose: smooth blend from Hip (`3.2°` natural cant) -> ADS (`ADS_OFFSET`).
    let mut gun_trans = [
        ADS_OFFSET[0] * aim,
        ADS_OFFSET[1] * aim,
        ADS_OFFSET[2] * aim,
    ];
    let mut gun_rot = [0.0, 0.0, 3.2 * hip];
    let gun_scale = 1.0 - 0.07 * aim;

    // Subtle procedural breathing + walk Lissajous sway (attenuated 88% in ADS).
    let sway_scale = 0.12 + 0.88 * hip;
    let breath_y = (idle_time * 1.75).sin() * 0.0028 * sway_scale;
    let breath_rx = (idle_time * 1.75 + 0.6).cos() * 0.35 * sway_scale;
    let bob_x = walk_phase.cos() * 0.014 * walk_speed * sway_scale;
    let bob_y = -(walk_phase * 2.0).sin().abs() * 0.012 * walk_speed * sway_scale;
    let bob_rz = walk_phase.sin() * 1.45 * walk_speed * sway_scale;

    gun_trans[0] += bob_x;
    gun_trans[1] += breath_y + bob_y;
    gun_rot[0] += breath_rx;
    gun_rot[2] += bob_rz;

    // Sprint low-ready carry across chest when not aiming or running an action animation.
    if sprinting && aim < 0.05 && mode == AnimMode::None {
        gun_trans[0] -= 0.11;
        gun_trans[1] -= 0.075;
        gun_trans[2] += 0.03;
        gun_rot[0] -= 12.0;
        gun_rot[1] += 26.0;
        gun_rot[2] += 14.0;
    }

    // Crisp backward/upward recoil kick (`+Z` toward shoulder, `+X` muzzle rise).
    if recoil > 0.001 {
        let kick_z = if aim > 0.5 { 0.075 } else { 0.095 };
        let kick_pitch = if aim > 0.5 { 4.8 } else { 8.0 };
        gun_trans[2] += recoil * kick_z;
        gun_trans[1] += recoil * 0.016 + flash * 0.005;
        gun_rot[0] += recoil * kick_pitch;
    }

    // Default articulated sub-part angles & hand anchors (in `[0..16]` JSON texels):
    // - Right hand grips wrist behind trigger guard (`[8.20, 8.20, 17.50]`).
    // - Left hand cradles walnut forestock (`guarda_mao_traseiro`, `[7.85, 9.55, 5.20]`).
    let mut lever_deg = 0.0f32;
    let mut hammer_deg = if hammer_uncocked { -36.0 } else { 0.0 };
    let mut trigger_deg = 0.0f32;
    let mut gate_deg = 0.0f32;
    let mut show_cartridge = false;
    let mut cartridge_tex = [11.2, 8.2, 11.2];
    let mut cartridge_rot = [0.0, 0.0, 0.0];

    let base_right_tex = [8.20, 8.20, 17.50];
    let base_right_rot = [
        112.0 + 2.0 * aim,
        12.0 - 2.0 * aim,
        -8.0 + 2.0 * aim,
    ];
    let base_left_tex = [7.85, 9.55, 5.20];
    let base_left_rot = [
        118.0 + 18.0 * aim,
        -22.0 + 2.0 * aim,
        16.0 - 2.0 * aim,
    ];

    let mut right_hand_tex = base_right_tex;
    let mut right_hand_rot = base_right_rot;
    let mut left_hand_tex = base_left_tex;
    let mut left_hand_rot = base_left_rot;

    match mode {
        AnimMode::None => {}
        AnimMode::Draw => {
            // Smooth TAC draw: rises from low right hip with subtle wrist roll settle.
            let p = smoothstep((mode_ticks / DRAW_TICKS).clamp(0.0, 1.0));
            let inv = 1.0 - p;
            gun_trans[0] += 0.08 * inv;
            gun_trans[1] -= 0.28 * inv;
            gun_trans[2] += 0.06 * inv;
            gun_rot[0] -= 26.0 * inv;
            gun_rot[1] -= 14.0 * inv;
            gun_rot[2] += 18.0 * inv;
        }
        AnimMode::FireCycle { keep_uncocked, .. } => {
            // Phase A (`0..3t`): trigger squeezed, hammer snaps forward (`0° -> -36°`) to strike firing pin.
            let strike = (mode_ticks / 1.1).clamp(0.0, 1.0);
            trigger_deg = -20.0 * (1.0 - ((mode_ticks - 1.2).max(0.0) / 3.0).clamp(0.0, 1.0));
            hammer_deg = -36.0 * strike;

            if !keep_uncocked && mode_ticks >= LEVER_DELAY_TICKS as f32 {
                // Phase B (`3..14t`): lever throws open (`+48°`), cocks hammer (`-36° -> +7° -> 0°`),
                // right hand follows lever loop forward/down and snaps it shut.
                let lt = ((mode_ticks - LEVER_DELAY_TICKS as f32) / LEVER_DURATION_TICKS as f32)
                    .clamp(0.0, 1.0);
                let lever_curve = if lt < 0.45 {
                    smoothstep(lt / 0.45)
                } else {
                    1.0 - smoothstep((lt - 0.45) / 0.55)
                };
                lever_deg = 48.0 * lever_curve;

                // Hammer pushed back past full-cock notch as bolt carrier slides rearward, then locks at `0°`.
                hammer_deg = if lt < 0.48 {
                    lerp(-36.0, 7.5, smoothstep(lt / 0.48))
                } else {
                    lerp(7.5, 0.0, smoothstep((lt - 0.48) / 0.52))
                };

                // Subtle clockwise wrist roll while working the lever loop (even in ADS).
                let roll_scale = 1.0 - 0.45 * aim;
                gun_rot[2] += 6.5 * lever_curve * roll_scale;
                gun_rot[0] -= 2.2 * lever_curve * roll_scale;

                right_hand_tex = [
                    base_right_tex[0],
                    base_right_tex[1] - 2.35 * lever_curve,
                    base_right_tex[2] - 2.15 * lever_curve,
                ];
                right_hand_rot[0] = base_right_rot[0] + 24.0 * lever_curve;
            }
        }
        AnimMode::DryClick => {
            let p = 1.0 - (mode_ticks / 5.0).clamp(0.0, 1.0);
            trigger_deg = -20.0 * p;
            hammer_deg = -36.0;
            gun_rot[0] -= 1.4 * p;
        }
        AnimMode::ReloadIntro { .. } => {
            // Rifle rolls `-20°` left to expose the right-side King's loading gate (`porta_B`),
            // while left hand slides back from the forestock toward the receiver!
            let p = smoothstep((mode_ticks / RELOAD_INTRO_TICKS as f32).clamp(0.0, 1.0));
            apply_reload_cradle(&mut gun_trans, &mut gun_rot, p);
            left_hand_tex = lerp3(base_left_tex, [10.45, 8.95, 11.40], p);
            left_hand_rot = lerp3(base_left_rot, [108.0, -10.0, 26.0], p);
        }
        AnimMode::ReloadLoop { .. } => {
            // Single `.44-40` cartridge insertion cycle (`0..10t`):
            //   0.00..0.32: left thumb brings brass cartridge up from belt to loading gate mouth
            //   0.32..0.72: nose presses gate inward (`gate_deg = -22°`) & slides forward into tube
            //   0.72..1.00: gate snaps shut with crisp receiver nudge, hand dips for next shell
            apply_reload_cradle(&mut gun_trans, &mut gun_rot, 1.0);
            let u = (mode_ticks / RELOAD_SHELL_TICKS as f32).clamp(0.0, 1.0);

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

            // Loading gate depresses inward (`-Y` rotation around `[8.936, 10.592, 10.432]`) as shell enters.
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

            // Subtle physical nudge on receiver as rim clicks past the magazine stop (`u ≈ 0.62`).
            let seat_bump = (-((u - 0.62) * 7.5).powi(2)).exp();
            gun_trans[0] -= 0.010 * seat_bump;
            gun_trans[1] += 0.008 * seat_bump;
            gun_rot[2] -= 2.4 * seat_bump;

            left_hand_tex = if u < 0.32 {
                lerp3([10.55, 8.75, 11.75], [9.85, 10.15, 11.65], smoothstep(u / 0.32))
            } else if u < 0.70 {
                lerp3(
                    [9.85, 10.15, 11.65],
                    [9.40, 10.30, 10.85],
                    smoothstep((u - 0.32) / 0.38),
                )
            } else {
                lerp3(
                    [9.40, 10.30, 10.85],
                    [10.55, 8.75, 11.75],
                    smoothstep((u - 0.70) / 0.30),
                )
            };
            left_hand_rot = [108.0, -12.0 + 6.0 * seat_bump, 26.0];
        }
        AnimMode::ReloadEndNorm => {
            let p = 1.0 - smoothstep((mode_ticks / RELOAD_END_NORM_TICKS as f32).clamp(0.0, 1.0));
            apply_reload_cradle(&mut gun_trans, &mut gun_rot, p);
            left_hand_tex = lerp3(base_left_tex, [10.45, 8.95, 11.40], p);
            left_hand_rot = lerp3(base_left_rot, [108.0, -10.0, 26.0], p);
        }
        AnimMode::ReloadEndEmpty { .. } => {
            // Empty-reload finale (`15t`):
            //   0..4t:  left hand returns to forestock while rifle rotates upright
            //   3..14t: right hand works the lever (`+50°`) to chamber the fresh `.44-40` round
            //           and cock the hammer (`-36° -> +8° -> 0°`)!
            let total = RELOAD_END_EMPTY_TICKS as f32;
            let settle = 1.0 - smoothstep((mode_ticks / 5.5).clamp(0.0, 1.0));
            apply_reload_cradle(&mut gun_trans, &mut gun_rot, settle);
            left_hand_tex = lerp3(base_left_tex, [10.45, 8.95, 11.40], settle);
            left_hand_rot = lerp3(base_left_rot, [108.0, -10.0, 26.0], settle);

            if mode_ticks >= 3.0 {
                let lt = ((mode_ticks - 3.0) / (total - 3.5)).clamp(0.0, 1.0);
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
                gun_trans[2] += 0.025 * lever_curve;
                right_hand_tex = [
                    base_right_tex[0],
                    base_right_tex[1] - 2.45 * lever_curve,
                    base_right_tex[2] - 2.25 * lever_curve,
                ];
                right_hand_rot[0] = base_right_rot[0] + 25.0 * lever_curve;
            }
        }
        AnimMode::Inspect => {
            // TAC-style 3-stage showcase (`68t` = `3.4s`):
            //   1. `0.00..0.42`: Tilt left to showcase engraved receiver & brass King's loading gate
            //   2. `0.42..0.82`: Roll over to show left receiver plate, walnut stock & octagon barrel
            //   3. `0.82..1.00`: Smoothly settle back into hip stance
            let u = (mode_ticks / INSPECT_TICKS).clamp(0.0, 1.0);
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
                let drift = ((u - 0.16) / 0.26 * core::f32::consts::PI).sin();
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
                let drift = ((u - 0.58) / 0.24 * core::f32::consts::PI).sin();
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
            left_hand_tex = lerp3(base_left_tex, [7.85, 9.45, 6.20], l_shift);
            left_hand_rot = lerp3(base_left_rot, [124.0, -18.0, 14.0], l_shift);
        }
    }

    WeaponPose {
        gun_trans,
        gun_rot,
        gun_scale,
        lever_deg,
        hammer_deg,
        trigger_deg,
        gate_deg,
        show_cartridge,
        cartridge_tex,
        cartridge_rot,
        right_hand_tex,
        right_hand_rot,
        left_hand_tex,
        left_hand_rot,
    }
}

fn apply_reload_cradle(gun_trans: &mut [f32; 3], gun_rot: &mut [f32; 3], weight: f32) {
    gun_trans[0] -= 0.075 * weight;
    gun_trans[1] -= 0.045 * weight;
    gun_trans[2] += 0.045 * weight;
    gun_rot[0] += 7.0 * weight;
    gun_rot[1] += 12.0 * weight;
    gun_rot[2] -= 21.0 * weight;
}

// ---------------------------------------------------------------------------
// 1st-Person Arm Drawing & Skin Fallback
// ---------------------------------------------------------------------------

fn resolve_client_skin(st: &mut ClientState, uuid: u128) -> ResolvedSkin {
    if let Some((srv_uuid, ref resolved)) = st.server_skin
        && srv_uuid == uuid
    {
        return resolved.clone();
    }
    if let Some((def_uuid, ref resolved)) = st.cached_default_uuid
        && def_uuid == uuid
    {
        return resolved.clone();
    }
    let (def_path, slim) = default_skin_for_uuid(uuid);
    let fallback = Identifier::parse(def_path)
        .unwrap_or_else(|_| Identifier::parse("entity/player/wide/steve").unwrap());
    let resolved = ResolvedSkin {
        primary: fallback.clone(),
        fallback,
        slim,
    };
    st.cached_default_uuid = Some((uuid, resolved.clone()));
    resolved
}

fn draw_gun_space_arm(
    draw: &mut BlockEntityDrawContext<'_>,
    arm_part: &ModelPart,
    skin: &ResolvedSkin,
    is_right_arm: bool,
    fist_tex: [f32; 3],
    rot_deg: [f32; 3],
) {
    // Center the rotation pivot on the arm's geometric center (`[-1, 9.5, 0]` wide or
    // `[-0.5, 9.5, 0]` slim for right arm; mirrored `+X` for left arm) so rotations never
    // skew between 3px slim and 4px wide skins.
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

    // Draw custom online skin (`skins/<sha1>`); if Horizon is still fetching the PNG over HTTP,
    // seamlessly draw the player's deterministic `default_skin(uuid)` until baking finishes.
    if draw.model_part(arm_part, &skin.primary, 0xFF_FF_FF_FF).is_err()
        && skin.primary != skin.fallback
    {
        let _ = draw.model_part(arm_part, &skin.fallback, 0xFF_FF_FF_FF);
    }
    draw.pop();
}

// ---------------------------------------------------------------------------
// HUD Helpers
// ---------------------------------------------------------------------------

fn draw_ads_vignette(gui: &mut GuiDraw<'_>, w: i32, h: i32, aim: f32) {
    let band_h = ((h as f32 * 0.14 * aim) as i32).max(1);
    let band_w = ((w as f32 * 0.10 * aim) as i32).max(1);
    let edge_alpha = ((aim * 84.0) as u32).min(115);
    let outer = edge_alpha << 24;

    gui.fill_gradient([0, 0], [w, band_h], outer, 0x0000_0000);
    gui.fill_gradient([0, h - band_h], [w, h], 0x0000_0000, outer);

    let mid_alpha = ((aim * 34.0) as u32) << 24;
    gui.fill([0, band_h], [band_w, h - band_h], mid_alpha);
    gui.fill([w - band_w, band_h], [w, h - band_h], mid_alpha);
}

fn draw_ammo_hud(gui: &mut GuiDraw<'_>, w: i32, h: i32, rounds: i32, reloading: bool) {
    let box_w = 54;
    let box_h = 16;
    let x0 = (w / 2 + 98).min(w - box_w - 6);
    let y0 = h - 20;

    gui.fill([x0, y0], [x0 + box_w, y0 + box_h], 0x94_14_11_0E);
    let border = if reloading {
        0xCC_E3_B3_4C
    } else if rounds == 0 {
        0xCC_C9_4A_38
    } else {
        0xAA_8C_6D_3F
    };
    gui.fill([x0, y0], [x0 + box_w, y0 + 1], border);
    gui.fill([x0, y0 + box_h - 1], [x0 + box_w, y0 + box_h], border);

    let label = if reloading {
        format!("{rounds}/{MAGAZINE_CAPACITY}+")
    } else {
        format!("{rounds} / {MAGAZINE_CAPACITY}")
    };
    let text_color = if rounds == 0 && !reloading {
        0xFF_FF_6E_59
    } else {
        0xFF_F5_E6_C8
    };
    gui.centered_text(
        &Text::literal(label),
        [x0 + box_w / 2, y0 + 4],
        text_color,
        true,
    );
}

// ---------------------------------------------------------------------------
// Network Payload Handlers
// ---------------------------------------------------------------------------

fn handle_rifle_input(msg: RifleInput, ctx: &mut dyn IPayloadContext) {
    let action = msg.action.0;
    ctx.enqueue_work(Box::new(move |host| {
        let HostSide::Server(host) = host else {
            return;
        };
        let Some(player) = host.players().acting_player() else {
            return;
        };
        match action {
            ACTION_FIRE => server_fire(host, player),
            ACTION_RELOAD => server_start_reload(host, player),
            ACTION_RELOAD_CANCEL => server_cancel_reload(host, player),
            ACTION_SKIN_REQUEST => server_sync_player_skin(host, player),
            _ => {}
        }
    }));
}

fn handle_rifle_sync(msg: RifleSync, ctx: &mut dyn IPayloadContext) {
    let kind = msg.kind.0;
    let rounds = msg.rounds.0.clamp(0, MAGAZINE_CAPACITY);

    ctx.enqueue_work(Box::new(move |host| {
        let HostSide::Client(host) = host else {
            return;
        };

        let mut sound_to_play: Option<(&'static str, f32, f32)> = None;
        {
            let mut st = CLIENT.lock().unwrap();
            st.known_rounds = rounds;
            match kind {
                SYNC_SHOT => {
                    let keep_uncocked = rounds == 0;
                    st.recoil = 1.0;
                    st.muzzle_flash = 1.0;
                    let pitch_kick = if st.aim_progress > 0.5 { 1.45 } else { 2.35 };
                    let yaw_sign = if (st.idle_time * 37.0).sin() >= 0.0 {
                        1.0
                    } else {
                        -1.0
                    };
                    st.camera_pitch_kick = (st.camera_pitch_kick + pitch_kick).min(4.8);
                    st.camera_yaw_kick += 0.32 * yaw_sign;
                    st.mode = AnimMode::FireCycle {
                        cock_sound_played: false,
                        lever_sound_played: false,
                        keep_uncocked,
                    };
                    st.mode_ticks = 0.0;
                }
                SYNC_DRY_CLICK => {
                    st.hammer_uncocked = true;
                    st.mode = AnimMode::DryClick;
                    st.mode_ticks = 0.0;
                    sound_to_play = Some((SND_COCK, 0.70, 1.32));
                }
                SYNC_RELOAD_START => {
                    let from_empty = rounds == 0;
                    st.aim_down = false;
                    st.mode = AnimMode::ReloadIntro { from_empty };
                    st.mode_ticks = 0.0;
                    sound_to_play = Some((SND_RELOAD_INTRO, 0.90, 1.0));
                }
                SYNC_SHELL_INSERTED => {
                    let from_empty = match st.mode {
                        AnimMode::ReloadIntro { from_empty }
                        | AnimMode::ReloadLoop { from_empty } => from_empty,
                        _ => false,
                    };
                    st.mode = AnimMode::ReloadLoop { from_empty };
                    st.mode_ticks = 0.0;
                    sound_to_play = Some((SND_RELOAD_INSERT, 0.92, 1.0));
                }
                SYNC_RELOAD_END_NORM => {
                    st.mode = AnimMode::ReloadEndNorm;
                    st.mode_ticks = 0.0;
                    sound_to_play = Some((SND_RELOAD_END, 0.88, 1.0));
                }
                SYNC_RELOAD_END_EMPTY => {
                    st.mode = AnimMode::ReloadEndEmpty {
                        lever_sound_played: false,
                        cock_sound_played: false,
                    };
                    st.mode_ticks = 0.0;
                    sound_to_play = Some((SND_RELOAD_END, 0.85, 1.0));
                }
                SYNC_RELOAD_ABORTED => {
                    if st.is_reloading() {
                        st.mode = AnimMode::None;
                        st.mode_ticks = 0.0;
                    }
                }
                _ => {}
            }
        }

        if let Some((snd, vol, pitch)) = sound_to_play {
            play_gun_sound(host, snd, vol, pitch);
        }
    }));
}
fn handle_rifle_skin_sync(msg: RifleSkinSync, ctx: &mut dyn IPayloadContext) {
    ctx.enqueue_work(Box::new(move |host| {
        let HostSide::Client(host) = host else {
            return;
        };
        let Some(player) = host.local_player() else {
            return;
        };
        let uuid = player.as_entity().uuid(host);
        let Ok(primary) = Identifier::parse(&msg.sprite) else {
            return;
        };
        let fallback = Identifier::parse(&msg.fallback).unwrap_or_else(|_| primary.clone());
        let slim = msg.slim.0 != 0;
        let mut st = CLIENT.lock().unwrap();
        st.server_skin = Some((
            uuid,
            ResolvedSkin {
                primary,
                fallback,
                slim,
            },
        ));
    }));
}

// ---------------------------------------------------------------------------
// Server-Authoritative Ballistics, Per-Round Reload & Skin Lookup
// ---------------------------------------------------------------------------

fn send_rifle_sync(host: &mut dyn ServerHost, player: horizon_mod_api::PlayerRef, kind: i32, rounds: i32) {
    let _ = host.send_to_player(
        player,
        handles().sync_channel,
        &RifleSync {
            kind: VarInt(kind),
            rounds: VarInt(rounds),
        },
    );
}

fn server_sync_player_skin(host: &mut dyn ServerHost, player: horizon_mod_api::PlayerRef) {
    let uuid = player.as_entity().uuid(host);
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

    let _ = host.send_to_player(
        player,
        handles().skin_channel,
        &RifleSkinSync {
            sprite,
            fallback: default_sprite.to_string(),
            slim: VarInt(i32::from(slim)),
        },
    );
}

fn server_cancel_reload(host: &mut dyn ServerHost, player: horizon_mod_api::PlayerRef) {
    let uuid = player.as_entity().uuid(host);
    let removed = SERVER.lock().unwrap().reloading.remove(&uuid).is_some();
    if removed {
        let stack = player.as_living().main_hand_item(host);
        let rounds = if is_winchester(&stack) {
            rounds_in_stack(&stack)
        } else {
            0
        };
        send_rifle_sync(host, player, SYNC_RELOAD_ABORTED, rounds);
    }
}

fn server_fire(host: &mut dyn ServerHost, shooter: horizon_mod_api::PlayerRef) {
    let living = shooter.as_living();
    let mut stack = living.main_hand_item(host);
    if !is_winchester(&stack) {
        return;
    }

    let uuid = shooter.as_entity().uuid(host);
    let creative = shooter.abilities(host).is_some_and(|a| a.instabuild);

    // Firing mid-reload interrupts the reload immediately so the shooter can fire loaded rounds!
    let (now, on_cooldown, was_reloading) = {
        let mut srv = SERVER.lock().unwrap();
        let now = srv.tick;
        let was_reloading = srv.reloading.remove(&uuid).is_some();
        let ready = srv.next_fire_tick.get(&uuid).copied().unwrap_or(0);
        if now < ready {
            (now, true, was_reloading)
        } else {
            (now, false, was_reloading)
        }
    };
    if on_cooldown {
        if was_reloading {
            send_rifle_sync(host, shooter, SYNC_RELOAD_ABORTED, rounds_in_stack(&stack));
        }
        return;
    }

    let rounds = rounds_in_stack(&stack);
    if rounds <= 0 && !creative {
        SERVER
            .lock()
            .unwrap()
            .next_fire_tick
            .insert(uuid, now + 6);
        send_rifle_sync(host, shooter, SYNC_DRY_CLICK, 0);
        return;
    }

    let remaining = if creative {
        rounds.max(1)
    } else {
        let next_rounds = rounds - 1;
        set_rounds_in_stack(&mut stack, next_rounds);
        let _ = living.set_main_hand_item(host, stack);
        next_rounds
    };

    let Some(level) = shooter.level(host) else {
        return;
    };
    let entity = shooter.as_entity();
    let Some(eye) = entity.eye_position(host) else {
        return;
    };
    let Some(look) = entity.look_angle(host) else {
        return;
    };

    let sprinting = entity.sprinting(host).unwrap_or(false);
    let on_ground = entity.on_ground(host).unwrap_or(true);
    let spread = if sprinting || !on_ground {
        HIP_SPREAD_RAD
    } else {
        0.0
    };
    let dir = apply_spread(look, spread, now ^ (uuid as u64));

    let muzzle = [
        eye[0] + dir[0] * 0.85,
        eye[1] - 0.10 + dir[1] * 0.85,
        eye[2] + dir[2] * 0.85,
    ];
    // World-audible `.44-40` gunshot + lever cycle for nearby players in multiplayer.
    let _ = host.play_sound(muzzle, SND_FIRE, SoundSource::Players, 2.4, 1.0);
    let _ = host.play_sound(muzzle, SND_THUMP, SoundSource::Players, 1.2, 1.0);
    let _ = level.spawn_particles(
        host,
        &ParticleOptions::simple(handles().smoke_particle),
        muzzle,
        8,
        [0.06, 0.06, 0.06],
        0.015,
    );
    let _ = level.spawn_particles(
        host,
        &ParticleOptions::simple(handles().flame_particle),
        muzzle,
        3,
        [0.02, 0.02, 0.02],
        0.01,
    );

    {
        let mut srv = SERVER.lock().unwrap();
        srv.next_fire_tick.insert(uuid, now + FIRE_COOLDOWN_TICKS);
    }

    send_rifle_sync(host, shooter, SYNC_SHOT, remaining);

    // Hitscan: solid-block terrain trace vs inflated LivingRef AABB trace.
    let block_dist = trace_blocks(host, level, eye, dir, RANGE_BLOCKS);
    let max_dist = block_dist.unwrap_or(RANGE_BLOCKS);

    if let Some((target, hit_pos)) = trace_entities(host, level, uuid, eye, dir, max_dist) {
        let _ = target.hurt_from_player(host, SHOT_DAMAGE, shooter);
        let _ = level.spawn_particles(
            host,
            &ParticleOptions::simple(handles().crit_particle),
            hit_pos,
            10,
            [0.12, 0.12, 0.12],
            0.08,
        );
    } else if let Some(d) = block_dist {
        let impact = [
            eye[0] + dir[0] * (d - 0.05),
            eye[1] + dir[1] * (d - 0.05),
            eye[2] + dir[2] * (d - 0.05),
        ];
        let _ = level.spawn_particles(
            host,
            &ParticleOptions::simple(handles().smoke_particle),
            impact,
            4,
            [0.04, 0.04, 0.04],
            0.01,
        );
    }
}

fn server_start_reload(host: &mut dyn ServerHost, player: horizon_mod_api::PlayerRef) {
    let living = player.as_living();
    let stack = living.main_hand_item(host);
    if !is_winchester(&stack) {
        return;
    }

    let current = rounds_in_stack(&stack);
    if current >= MAGAZINE_CAPACITY {
        return;
    }

    let creative = player.abilities(host).is_some_and(|a| a.instabuild);
    if !creative && find_ammo_slot(host, player).is_none() {
        send_rifle_sync(host, player, SYNC_DRY_CLICK, current);
        return;
    }

    let uuid = player.as_entity().uuid(host);
    let slot = player.selected_slot(host);
    let started = {
        let mut srv = SERVER.lock().unwrap();
        if srv.reloading.contains_key(&uuid) {
            false
        } else {
            let now = srv.tick;
            srv.reloading.insert(
                uuid,
                ReloadTracker {
                    slot,
                    next_insert_tick: now + RELOAD_INTRO_TICKS,
                    from_empty: current == 0,
                    inserted_any: false,
                    finishing_until: None,
                },
            );
            true
        }
    };

    if started {
        send_rifle_sync(host, player, SYNC_RELOAD_START, current);
    }
}

fn step_player_reload(
    host: &mut dyn ServerHost,
    uuid: u128,
    mut tracker: ReloadTracker,
    now: u64,
) {
    let Some(player) = host.players().by_uuid(EntityUuid::from_u128(uuid)) else {
        SERVER.lock().unwrap().reloading.remove(&uuid);
        return;
    };

    // Cancel if the player swapped hotbar slots or dropped the Winchester.
    let stack = player.as_living().main_hand_item(host);
    if player.selected_slot(host) != tracker.slot || !is_winchester(&stack) {
        SERVER.lock().unwrap().reloading.remove(&uuid);
        send_rifle_sync(host, player, SYNC_RELOAD_ABORTED, 0);
        return;
    }

    if let Some((end_tick, empty_chamber)) = tracker.finishing_until {
        if now >= end_tick {
            let mut srv = SERVER.lock().unwrap();
            srv.reloading.remove(&uuid);
            if empty_chamber {
                srv.next_fire_tick.insert(uuid, now + 2);
            }
        }
        return;
    }

    if now < tracker.next_insert_tick {
        return;
    }

    let mut stack = stack;
    let current = rounds_in_stack(&stack);
    let creative = player.abilities(host).is_some_and(|a| a.instabuild);

    if current >= MAGAZINE_CAPACITY || (!creative && !consume_one_cartridge(host, player)) {
        finish_reload_sequence(host, player, uuid, tracker, current, now);
        return;
    }

    let updated = (current + 1).min(MAGAZINE_CAPACITY);
    set_rounds_in_stack(&mut stack, updated);
    let _ = player.as_living().set_main_hand_item(host, stack);
    tracker.inserted_any = true;

    send_rifle_sync(host, player, SYNC_SHELL_INSERTED, updated);

    tracker.next_insert_tick = now + RELOAD_SHELL_TICKS;
    SERVER.lock().unwrap().reloading.insert(uuid, tracker);
}

fn finish_reload_sequence(
    host: &mut dyn ServerHost,
    player: horizon_mod_api::PlayerRef,
    uuid: u128,
    mut tracker: ReloadTracker,
    rounds: i32,
    now: u64,
) {
    if tracker.from_empty && tracker.inserted_any && rounds > 0 {
        let end_tick = now + RELOAD_END_EMPTY_TICKS;
        tracker.finishing_until = Some((end_tick, true));
        {
            let mut srv = SERVER.lock().unwrap();
            srv.reloading.insert(uuid, tracker);
            srv.next_fire_tick.insert(uuid, end_tick);
        }
        send_rifle_sync(host, player, SYNC_RELOAD_END_EMPTY, rounds);
    } else {
        let end_tick = now + RELOAD_END_NORM_TICKS;
        tracker.finishing_until = Some((end_tick, false));
        {
            let mut srv = SERVER.lock().unwrap();
            srv.reloading.insert(uuid, tracker);
            srv.next_fire_tick.insert(uuid, now + 4);
        }
        send_rifle_sync(host, player, SYNC_RELOAD_END_NORM, rounds);
    }
}

fn find_ammo_slot(host: &dyn ServerHost, player: horizon_mod_api::PlayerRef) -> Option<usize> {
    let ammo_id = handles().ammo.id();
    let size = player.inventory_size(host);
    (0..size).find(|&slot| {
        let s = player.inventory_item(host, slot);
        !s.is_empty() && s.item() == ammo_id
    })
}

fn consume_one_cartridge(host: &mut dyn ServerHost, player: horizon_mod_api::PlayerRef) -> bool {
    let Some(slot) = find_ammo_slot(host, player) else {
        return false;
    };
    let mut ammo_stack = player.inventory_item(host, slot);
    let count = ammo_stack.count();
    if count <= 0 {
        return false;
    }
    if count == 1 {
        let _ = player.set_inventory_item(host, slot, ItemStack::EMPTY);
    } else {
        ammo_stack.set_count(count - 1);
        let _ = player.set_inventory_item(host, slot, ammo_stack);
    }
    true
}

// ---------------------------------------------------------------------------
// Raycasting & Math Helpers
// ---------------------------------------------------------------------------

fn trace_blocks(
    host: &dyn ServerHost,
    level: horizon_mod_api::LevelRef,
    origin: [f64; 3],
    dir: [f64; 3],
    max_range: f64,
) -> Option<f64> {
    let mut t = 0.0;
    let mut last_block = [i32::MIN, i32::MIN, i32::MIN];
    while t <= max_range {
        let p = [
            origin[0] + dir[0] * t,
            origin[1] + dir[1] * t,
            origin[2] + dir[2] * t,
        ];
        let bp = [
            p[0].floor() as i32,
            p[1].floor() as i32,
            p[2].floor() as i32,
        ];
        if bp != last_block {
            last_block = bp;
            let state = level.block_state(host, bp);
            if !state.is_air(host) && state.has_collision(host, level, bp) {
                return Some(t);
            }
        }
        t += BLOCK_RAY_STEP;
    }
    None
}

fn trace_entities(
    host: &dyn ServerHost,
    level: horizon_mod_api::LevelRef,
    shooter_uuid: u128,
    origin: [f64; 3],
    dir: [f64; 3],
    max_dist: f64,
) -> Option<(LivingRef, [f64; 3])> {
    let end = [
        origin[0] + dir[0] * max_dist,
        origin[1] + dir[1] * max_dist,
        origin[2] + dir[2] * max_dist,
    ];
    let min = [
        origin[0].min(end[0]) - 1.0,
        origin[1].min(end[1]) - 1.0,
        origin[2].min(end[2]) - 1.0,
    ];
    let max = [
        origin[0].max(end[0]) + 1.0,
        origin[1].max(end[1]) + 1.0,
        origin[2].max(end[2]) + 1.0,
    ];

    let candidates = level.living_entities_in_aabb(host, min, max);
    let mut closest: Option<(f64, LivingRef)> = None;

    for living in candidates {
        let ent = living.as_entity();
        if ent.uuid(host) == shooter_uuid || !living.alive(host) {
            continue;
        }
        let Some((bmin, bmax)) = ent.bounding_box(host) else {
            continue;
        };
        let inflated_min = [
            bmin[0] - ENTITY_HIT_INFLATE,
            bmin[1] - ENTITY_HIT_INFLATE,
            bmin[2] - ENTITY_HIT_INFLATE,
        ];
        let inflated_max = [
            bmax[0] + ENTITY_HIT_INFLATE,
            bmax[1] + ENTITY_HIT_INFLATE,
            bmax[2] + ENTITY_HIT_INFLATE,
        ];
        if let Some(hit_t) = ray_aabb(origin, dir, inflated_min, inflated_max)
            && hit_t <= max_dist
            && closest.is_none_or(|(best, _)| hit_t < best)
        {
            closest = Some((hit_t, living));
        }
    }

    closest.map(|(t, living)| {
        let hit = [
            origin[0] + dir[0] * t,
            origin[1] + dir[1] * t,
            origin[2] + dir[2] * t,
        ];
        (living, hit)
    })
}

fn ray_aabb(origin: [f64; 3], dir: [f64; 3], min: [f64; 3], max: [f64; 3]) -> Option<f64> {
    let mut tmin = 0.0_f64;
    let mut tmax = f64::INFINITY;
    for i in 0..3 {
        if dir[i].abs() < 1e-9 {
            if origin[i] < min[i] || origin[i] > max[i] {
                return None;
            }
        } else {
            let inv = 1.0 / dir[i];
            let mut t1 = (min[i] - origin[i]) * inv;
            let mut t2 = (max[i] - origin[i]) * inv;
            if t1 > t2 {
                core::mem::swap(&mut t1, &mut t2);
            }
            tmin = tmin.max(t1);
            tmax = tmax.min(t2);
            if tmin > tmax {
                return None;
            }
        }
    }
    Some(tmin)
}

fn apply_spread(look: [f64; 3], spread_rad: f64, seed: u64) -> [f64; 3] {
    if spread_rad <= 0.0 {
        return normalize(look);
    }
    let h1 = splitmix64(seed);
    let h2 = splitmix64(h1);
    let u1 = ((h1 >> 11) as f64) * (1.0 / ((1u64 << 53) as f64)) * 2.0 - 1.0;
    let u2 = ((h2 >> 11) as f64) * (1.0 / ((1u64 << 53) as f64)) * 2.0 - 1.0;
    let perturbed = [
        look[0] + u1 * spread_rad,
        look[1] + u2 * spread_rad,
        look[2] + (u1 - u2) * 0.5 * spread_rad,
    ];
    normalize(perturbed)
}

fn normalize(v: [f64; 3]) -> [f64; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len < 1e-9 {
        [0.0, 0.0, 1.0]
    } else {
        [v[0] / len, v[1] / len, v[2] / len]
    }
}

const fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
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
