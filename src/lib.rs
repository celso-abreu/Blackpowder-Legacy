//! Blackpowder Legacy — Winchester 1873 lever-action rifle (wasm mod).
//!
//! Controls (v0.6):
//! - Hold right click: shoulder the rifle and aim down the sights — the gun
//!   slides to the screen's centre (`render_hand`), the view zooms in
//!   smoothly (`compute_fov_modifier`) and a vanilla vignette frames it
//!   (`gui_render_post`). Aiming for at least LEVER_TICKS cycles the lever.
//! - Left click while aiming: fire one round (`key_mapping_pre` cancels the
//!   vanilla attack, so the rifle never mines or punches, and a typed payload
//!   asks the server to shoot). Each shot kicks (recoil) and the lever cycles
//!   again: LEVER_TICKS between shots, enforced server-side, with the gun
//!   rolling through the lever motion in first person.
//! - R (a real Controls keybind, remappable): load one cartridge per press,
//!   RELOAD_TICKS apart, up to the tube's 8 — the gun dips as each round
//!   slides in.
//!
//! The server stays authoritative: tube load, lever timing, cartridge
//! consumption and the projectile all live server-side. The client only
//! detects inputs and draws the aiming effects, so aim visuals never lag.
//!
//! The magazine lives in the stack's vanilla `damage` component (0..=8);
//! the item declares `max_damage = 100` so every write is valid and the rifle
//! never breaks. The `item_bar` hook replaces the (misleading) vanilla damage
//! bar with a brass ammo gauge. Sounds go through the level's own broadcast
//! (`LevelRef::play_sound`), never /playsound commands (those spammed the
//! chat). A shot also puffs black-powder smoke past the muzzle.

use mod_api::{
    Blend, ClientHost, ClientTickPreEvent, ComputeFovModifierEvent, GuiDraw, GuiRenderPostEvent,
    Hand, Hooks, Host, HostSide, Identifier, IPayloadContext, ItemBarEvent, ItemKey, ItemPart,
    ItemStack, KeyConflictContext, KeyMappingPreEvent, KeyRegistration, LivingRef, OnUseTickEvent,
    ParticleKey, ParticleOptions, PayloadId, Registrar, RegistrationContext, ReleaseUsingEvent,
    RenderHandEvent, ServerHost, ServerTickEvent, ServerTickPhase, SoundSource, Text, ToServer,
    UseDurationEvent, VarInt,
};
use std::collections::BTreeMap;
use std::f32::consts::PI;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

const WINCHESTER: &str = "blackpowder_legacy:winchester_1873";
const CARTRIDGE: &str = "blackpowder_legacy:cartridge_44";
const BULLET: &str = "blackpowder_legacy:bullet";

const TUBE_CAPACITY: u32 = 8;
const USE_DURATION: i32 = 72000; // vanilla's "use forever" duration
const LEVER_TICKS: i32 = 8; // 0.4 s: aim ticks cycling the lever, and the delay between shots
const RELOAD_TICKS: u64 = 6; // 0.3 s between one cartridge sliding in and the next

/// The mod's own id (horizon-mod.toml), owning the registered keybind.
const MOD_ID: &str = "blackpowder_legacy";
/// The registered reload keybind's id within the mod (`Options > Controls`).
const RELOAD_MAPPING: &str = "reload";
const ATTACK_MAPPING: &str = "key.attack";

/// The aim zoom's deepest FOV scale (smaller = narrower); eased toward it.
const AIM_FOV: f32 = 0.6;
/// How far the vignette darkens at full aim (1.0 is vanilla's invisible).
const AIM_VIGNETTE: f32 = 0.65;

// First-person animation, in client ticks (matched by the server's delays).
const LEVER_ANIM_TICKS: f32 = 8.0;
const RELOAD_ANIM_TICKS: f32 = 6.0;
/// Where the sights go at full aim, in view-space blocks (+x right, +y up,
/// -z into the screen): the sight line leaves (0.56, -0.40) for the camera
/// axis, the gun pulled a touch closer.
const ADS_OFFSET: [f32; 3] = [-0.56, 0.38, 0.05];

// Vanilla sound events; played through the level broadcast, never via commands.
const SHOT_SOUND: &str = "minecraft:entity.firework_rocket.blast";
const DRY_SOUND: &str = "minecraft:block.lever.click";
const LOAD_SOUND: &str = "minecraft:item.crossbow.loading_middle";
const READY_SOUND: &str = "minecraft:item.crossbow.loading_end";

// Volume above 1.0 extends the hearing range past 16 blocks (3.0 → ~48, as a
// gunshot should carry); the rest stay inside the vanilla 16-block bubble.
const SHOT_VOLUME: f32 = 3.0;
const SHOT_PITCH: f32 = 0.6;
const UI_VOLUME: f32 = 0.7;
const DRY_PITCH: f32 = 0.7;
const LOAD_PITCH: f32 = 1.2;
const READY_PITCH: f32 = 1.0;
/// The lever click when the post-shot cycle completes (from the aim ticker's
/// server half, LEVER_TICKS after a shot).
const LEVER_SOUND: &str = "minecraft:item.crossbow.loading_end";
const LEVER_PITCH: f32 = 1.5;

const SMOKE_PARTICLE: &str = "minecraft:large_smoke";
/// The ammo gauge's colour (0xRRGGBB): brass, like the cartridge cases.
const AMMO_BAR_RGB: u32 = 0xC9_A2_27;

// Rifle input actions carried by the typed payload.
const ACTION_FIRE: i32 = 1;
const ACTION_RELOAD: i32 = 2;

/// Client-side aim state: set while the rifle is in use (aimed), cleared on
/// release. Drives the zoom, the vignette and the left-click fire gate.
static AIMING: AtomicBool = AtomicBool::new(false);
/// A fire input queued by `key_mapping_pre` for the next client tick's send.
static PENDING_FIRE: AtomicBool = AtomicBool::new(false);

/// Client tick counter, bumped by `client_tick_pre`.
static CLIENT_TICKS: AtomicU64 = AtomicU64::new(0);
/// The last client tick `on_use_tick` saw the rifle in use. Dropping or
/// swapping the held item stops the use *without* a release, so the heartbeat
/// — not `release_using` alone — is what reliably ends an aim.
static AIM_LAST_TICK: AtomicU64 = AtomicU64::new(0);
/// The eased aim strength (f32 bits, 0.0..=1.0), advanced per frame by
/// `compute_fov_modifier`; drives the zoom, the vignette and the gun's slide
/// to the sights.
static ADS_BITS: AtomicU32 = AtomicU32::new(0);
/// The client tick the shot cycle (recoil + lever) animation started, or
/// u64::MAX when idle.
static LEVER_ANIM_START: AtomicU64 = AtomicU64::new(u64::MAX);
/// The client tick the reload dip animation started, or u64::MAX when idle.
static RELOAD_ANIM_START: AtomicU64 = AtomicU64::new(u64::MAX);

/// Server tick counter, bumped by `server_tick` (end phase).
static SERVER_TICKS: AtomicU64 = AtomicU64::new(0);
/// Per-player lever state, server-side: (aim ticks, last server tick seen).
/// Written by `on_use_tick`, cleared by `release_using` and reset by each
/// shot. The timestamp keeps a dropped or swapped rifle from leaving a
/// "lever armed" state behind — a shot is only honoured while the use is
/// still ticking.
static AIM_TICKS: Mutex<BTreeMap<u128, (i32, u64)>> = Mutex::new(BTreeMap::new());
/// Per-player server tick at which the lever is worked through after the last
/// shot — the enforced delay between shots (the old AIM_TICKS reset alone was
/// overwritten by the next `on_use_tick`, so shots used to come out as fast
/// as the clicks arrived).
static LEVER_READY: Mutex<BTreeMap<u128, u64>> = Mutex::new(BTreeMap::new());
/// Per-player server tick at which the next cartridge may slide in.
static RELOAD_READY: Mutex<BTreeMap<u128, u64>> = Mutex::new(BTreeMap::new());

/// The registered serverbound channel for rifle inputs.
static RIFLE_INPUT: OnceLock<ToServer<RifleInput>> = OnceLock::new();
/// The rifle's item key, cached so `render_hand` can recognise the held stack
/// without a registry lookup per frame.
static RIFLE_KEY: OnceLock<ItemKey> = OnceLock::new();
/// The smoke particle key, kept for the payload handler (the Part owns one
/// copy; the free-function handler reaches this one).
static SMOKE_KEY: OnceLock<ParticleKey> = OnceLock::new();

/// One rifle input from the owning client: fire (1) or reload one round (2).
#[derive(horizon_mod_api_macros::PayloadCodec)]
struct RifleInput {
    action: VarInt,
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
        // The reload key as a real Controls binding: remappable, persisted in
        // horizon-mods.txt and silent while a screen or the chat is open
        // (which the raw `input_key` listener was not).
        r.register_key_mapping(
            KeyRegistration::new(RELOAD_MAPPING, "key.keyboard.r")
                .label("key.blackpowder_legacy.reload")
                .context(KeyConflictContext::InGame),
        );
        let mut payloads = r.payload_handlers();
        let mut view = payloads.registrar("1");
        let input = view.play_to_server_typed::<RifleInput>(
            PayloadId("blackpowder_legacy:rifle_input".into()),
            on_rifle_input,
        );
        let _ = RIFLE_INPUT.set(input);
    }

    /// Left click while aiming fires: cancel the vanilla attack press (no
    /// mining, no punch) and queue the shot. Outside an aim the attack
    /// behaves as usual, so carrying the rifle never blocks the pickaxe use.
    fn key_mapping_pre(
        &self,
        _: &mut dyn ClientHost,
        event: &mut KeyMappingPreEvent<'_>,
    ) -> mod_api::Flow {
        if event.mapping == ATTACK_MAPPING
            && event.pressed
            && AIMING.load(Ordering::Relaxed)
        {
            PENDING_FIRE.store(true, Ordering::Relaxed);
            return mod_api::Flow::Cancel;
        }
        mod_api::Flow::Continue
    }

    /// The client tick: heartbeat for the aim flag, then the payload sends
    /// (they need this mutable client host).
    fn client_tick_pre(&self, host: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
        // End a stale aim: dropping or swapping the rifle stops its use
        // without a release, which used to leave the vignette and the fire
        // gate stuck on.
        let tick = CLIENT_TICKS.fetch_add(1, Ordering::Relaxed) + 1;
        if AIMING.load(Ordering::Relaxed)
            && tick - AIM_LAST_TICK.load(Ordering::Relaxed) > 2
        {
            AIMING.store(false, Ordering::Relaxed);
        }
        let Some(input) = RIFLE_INPUT.get() else { return };
        // One reload payload per queued keybind click; clicks that happened
        // behind an open screen never reach us. Each sent press starts the
        // reload dip; the server paces the cartridges (RELOAD_TICKS).
        while host.consume_click(MOD_ID, RELOAD_MAPPING) {
            let _ = input.send(
                host,
                &RifleInput {
                    action: VarInt(ACTION_RELOAD),
                },
            );
            RELOAD_ANIM_START.store(tick, Ordering::Relaxed);
        }
        if PENDING_FIRE.swap(false, Ordering::Relaxed) {
            let _ = input.send(
                host,
                &RifleInput {
                    action: VarInt(ACTION_FIRE),
                },
            );
            // The recoil + lever cycle plays on send; if the server refuses
            // the shot (lever mid-cycle, empty tube) the dip still reads as
            // the attempt.
            LEVER_ANIM_START.store(tick, Ordering::Relaxed);
        }
    }

    /// The server tick feeds the lever state's clock.
    fn server_tick(&self, _: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
        if matches!(event.phase, ServerTickPhase::ServerTickEnd { .. }) {
            SERVER_TICKS.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The aim zoom: the FOV scale eases toward AIM_FOV as the sights come
    /// up, and back out as the aim drops — one shared eased value
    /// (`ADS_BITS`), advanced here once per frame, also drives the vignette
    /// and the gun's slide in `render_hand`.
    fn compute_fov_modifier(&self, _: &dyn ClientHost, event: &mut ComputeFovModifierEvent<'_>) {
        let target: f32 = if AIMING.load(Ordering::Relaxed) { 1.0 } else { 0.0 };
        let value = f32::from_bits(ADS_BITS.load(Ordering::Relaxed));
        let value = value + (target - value) * 0.3;
        ADS_BITS.store(value.to_bits(), Ordering::Relaxed);
        if value > 0.001 {
            event.modifier = 1.0 - (1.0 - AIM_FOV) * value;
        }
    }

    /// First-person pose of the rifle, per frame: at full aim the sights sit
    /// on the camera axis (ADS_OFFSET); a shot adds a recoil kick that decays
    /// into the lever's roll-and-dip cycle; each cartridge loaded dips the
    /// gun. `render`'s transform is view-space (+x right, +y up, -z into the
    /// screen), applied before the hand's own pose, so the offsets compose
    /// with the item's display transform directly.
    fn render_hand(&self, _: &dyn ClientHost, event: &mut RenderHandEvent<'_>) -> mod_api::Flow {
        let render = &mut *event.render;
        if render.hand != Hand::Main {
            return mod_api::Flow::Continue;
        }
        let Some(rifle) = RIFLE_KEY.get().copied() else {
            return mod_api::Flow::Continue;
        };
        if !matches!(render.stack.map(|stack| stack.item()), Some(item) if item == rifle) {
            return mod_api::Flow::Continue;
        }
        let aim = f32::from_bits(ADS_BITS.load(Ordering::Relaxed));
        let mut offset = [ADS_OFFSET[0] * aim, ADS_OFFSET[1] * aim, ADS_OFFSET[2] * aim];
        let mut rotate = [0.0f32; 3];
        let now = CLIENT_TICKS.load(Ordering::Relaxed);
        // The shot cycle: a fast recoil kick, then the lever's roll-and-dip.
        let lever = LEVER_ANIM_START.load(Ordering::Relaxed);
        if lever != u64::MAX {
            let p = (now.saturating_sub(lever) as f32 + render.partial) / LEVER_ANIM_TICKS;
            if p >= 1.0 {
                LEVER_ANIM_START.store(u64::MAX, Ordering::Relaxed);
            } else {
                let recoil = (-6.0 * p).exp();
                offset[2] += 0.10 * recoil; // kicked back toward the shoulder
                rotate[0] += 7.0 * recoil; // muzzle rise
                let dip = (((p - 0.2) / 0.8).clamp(0.0, 1.0) * PI).sin();
                rotate[2] += 10.0 * dip; // canted right as the lever swings
                offset[1] -= 0.05 * dip;
            }
        }
        // One cartridge in: a short dip-and-roll.
        let reload = RELOAD_ANIM_START.load(Ordering::Relaxed);
        if reload != u64::MAX {
            let p = (now.saturating_sub(reload) as f32 + render.partial) / RELOAD_ANIM_TICKS;
            if p >= 1.0 {
                RELOAD_ANIM_START.store(u64::MAX, Ordering::Relaxed);
            } else {
                let dip = (p * PI).sin();
                offset[1] -= 0.07 * dip;
                offset[0] += 0.02 * dip;
                rotate[2] += 8.0 * dip;
            }
        }
        for axis in 0..3 {
            render.translation[axis] += offset[axis];
            render.rotation[axis] += rotate[axis];
        }
        mod_api::Flow::Continue
    }

    /// The aim vignette: vanilla's own `misc/vignette` texture blitted full
    /// screen with the VIGNETTE blend (dst * (1 - src)), exactly how the game
    /// frames dark areas — the eased aim strength scales its brightness, so
    /// the edges close in as the sights rise. Hidden with the F1 HUD.
    fn gui_render_post(&self, _: &dyn ClientHost, event: &mut GuiRenderPostEvent<'_>) {
        let aim = f32::from_bits(ADS_BITS.load(Ordering::Relaxed));
        if aim < 0.02 || event.gui_hidden {
            return;
        }
        let [w, h] = event.batch.size();
        let brightness = ((1.0 - (1.0 - AIM_VIGNETTE) * aim) * 255.0) as u32;
        let color = 0xFF00_0000 | (brightness << 16) | (brightness << 8) | brightness;
        let (w, h) = (w as i32, h as i32);
        let _ = event.batch.gui_paint(&[GuiDraw::Blit {
            texture: "misc/vignette".into(),
            blend: Blend::Vignette,
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

/// The server's answer to one rifle input. The context carries the sending
/// player and the server host on the main thread.
fn on_rifle_input(value: RifleInput, cx: &mut dyn IPayloadContext) -> Result<(), String> {
    let player = cx.player().map_err(|e| format!("{e:?}"))?;
    let host = cx.server_host().map_err(|e| format!("{e:?}"))?;
    match value.action.0 {
        ACTION_FIRE => {
            let Some(smoke) = SMOKE_KEY.get().copied() else {
                return Ok(());
            };
            fire_rifle(host, player, smoke);
        }
        ACTION_RELOAD => reload_one(host, player),
        _ => {}
    }
    Ok(())
}

struct Winchester {
    smoke: ParticleKey,
}

/// Rounds in the tube, read from the stack's damage component.
fn rounds(stack: &ItemStack) -> u32 {
    stack.damage().min(TUBE_CAPACITY)
}

/// The acting player of a server hook (the one using the rifle), or none when
/// the call has no acting player.
fn shooter(host: &dyn ServerHost) -> Option<mod_api::PlayerRef> {
    host.players().acting_player()
}

/// Play a sound at the player through the level's own broadcast
/// (`ServerLevel.playSound(null, ...)`): every nearby player hears it and no
/// command feedback reaches anyone's chat.
fn play_sound_at(host: &mut dyn ServerHost, player: mod_api::PlayerRef, sound: &str, volume: f32, pitch: f32) {
    let entity = player.entity();
    let Some(level) = entity.level(host) else {
        return;
    };
    let Some(at) = entity.position(host) else {
        return;
    };
    level.play_sound(host, sound, at, SoundSource::Players, volume, pitch);
}

/// A puff of black-powder smoke just past the muzzle, along the view vector.
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

/// The first rifle in the player's inventory, with its slot.
fn find_rifle(host: &mut dyn ServerHost, player: mod_api::PlayerRef) -> Option<(u32, ItemStack)> {
    let slots = player.inventory_slots(host)?;
    for (slot, stack) in slots {
        if stack.item().id(host).to_string() == WINCHESTER {
            return Some((slot, stack));
        }
    }
    None
}

/// Take one cartridge from the player's inventory. Returns true when taken.
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

/// Load one cartridge into the tube (one R press, RELOAD_TICKS apart), with
/// the loading sounds and the ammo counter. Server-side.
fn reload_one(host: &mut dyn ServerHost, player: mod_api::PlayerRef) {
    let uuid = player.uuid();
    let now = SERVER_TICKS.load(Ordering::Relaxed);
    let reload_ready = RELOAD_READY
        .lock()
        .unwrap()
        .get(&uuid)
        .copied()
        .unwrap_or(0);
    if reload_ready > now {
        return;
    }
    let Some((slot, rifle)) = find_rifle(host, player) else {
        return;
    };
    let loaded = rounds(&rifle);
    if loaded >= TUBE_CAPACITY || !take_cartridge(host, player) {
        return;
    }
    RELOAD_READY.lock().unwrap().insert(uuid, now + RELOAD_TICKS);
    let mut rifle = rifle;
    rifle.set_damage(loaded + 1);
    if player.set_inventory_slot(host, slot, Some(rifle)).is_err() {
        return;
    }
    let full = loaded + 1 == TUBE_CAPACITY;
    if full {
        play_sound_at(host, player, READY_SOUND, UI_VOLUME, READY_PITCH);
    } else {
        play_sound_at(host, player, LOAD_SOUND, UI_VOLUME, LOAD_PITCH);
    }
    let _ = player.action_bar(
        host,
        Text::literal(format!("{} / {}", loaded + 1, TUBE_CAPACITY)),
    );
}

/// Fire one round, if the lever was cycled (aimed LEVER_TICKS, with the use
/// still ticking) and the tube is loaded. Cycling starts over after each
/// shot. Server-side.
fn fire_rifle(host: &mut dyn ServerHost, player: mod_api::PlayerRef, smoke: ParticleKey) {
    let uuid = player.uuid();
    let now = SERVER_TICKS.load(Ordering::Relaxed);
    let (aimed, last_seen) = AIM_TICKS
        .lock()
        .unwrap()
        .get(&uuid)
        .copied()
        .unwrap_or((0, 0));
    if aimed < LEVER_TICKS || now - last_seen > 2 {
        return; // the lever is still cycling, or the aim went stale (dropped)
    }
    // The post-shot lever delay. AIM_TICKS alone could never hold it: the
    // next on_use_tick overwrote the reset, so a fast clicker outpaced the
    // lever. LEVER_READY stands between shots.
    let lever_ready = LEVER_READY
        .lock()
        .unwrap()
        .get(&uuid)
        .copied()
        .unwrap_or(0);
    if lever_ready > now {
        return;
    }
    let Some((slot, rifle)) = find_rifle(host, player) else {
        return;
    };
    let loaded = rounds(&rifle);
    if loaded == 0 {
        play_sound_at(host, player, DRY_SOUND, UI_VOLUME, DRY_PITCH);
        return;
    }
    let Ok(bullet) = Identifier::parse(BULLET) else { return };
    let Some(item) = host.item_key(&bullet) else { return };
    let Some(carried) = ItemStack::new(item, 1) else {
        return;
    };
    let owner = LivingRef::new(player.entity());
    // Rifle shot: fast and flat; snowball-template ballistics apply.
    let shot = host.spawn_projectile_from_rotation(owner, &bullet, &carried, 0.0, 6.0, 0.35);
    if shot.is_none() {
        return;
    }
    let mut rifle = rifle;
    rifle.set_damage(loaded - 1);
    let _ = player.set_inventory_slot(host, slot, Some(rifle));
    AIM_TICKS.lock().unwrap().insert(uuid, (0, now));
    LEVER_READY.lock().unwrap().insert(uuid, now + LEVER_TICKS as u64);
    play_sound_at(host, player, SHOT_SOUND, SHOT_VOLUME, SHOT_PITCH);
    smoke_at_muzzle(host, player, smoke);
    let _ = player.action_bar(
        host,
        Text::literal(format!("{} / {}", loaded - 1, TUBE_CAPACITY)),
    );
}



#[mod_api::horizon_part]
impl ItemPart for Winchester {
    /// A use duration above zero is what makes a mod item usable over time
    /// on a right click (vanilla's `Item.use` → `startUsingItem`). For the
    /// rifle, "in use" is the aim.
    fn use_duration(&self, _: &dyn Host, event: &mut UseDurationEvent<'_>) {
        event.duration = USE_DURATION;
    }

    /// Ammo gauge under the rifle icon. Vanilla's damage bar would lie here:
    /// our "damage" component is the tube's load (0..=8) on a max_damage=100
    /// item, so it would show a nearly full green bar on a full tube and none
    /// when empty. Replace it with the tube's fullness, in brass.
    fn item_bar(&self, _: &dyn ClientHost, event: &mut ItemBarEvent<'_>) {
        let rounds = rounds(&event.stack);
        let width = (rounds * 13 / TUBE_CAPACITY) as i32;
        *event.bar = Some((width, AMMO_BAR_RGB));
    }

    /// While aimed: the client flags the aim (zoom, vignette, fire gate) and
    /// stamps the heartbeat; the server counts lever ticks with a timestamp.
    fn on_use_tick(&self, host: HostSide<'_>, event: &mut OnUseTickEvent<'_>) {
        match host {
            HostSide::Client(_) => {
                AIMING.store(true, Ordering::Relaxed);
                AIM_LAST_TICK.store(CLIENT_TICKS.load(Ordering::Relaxed), Ordering::Relaxed);
            }
            HostSide::Server(host) => {
                let Some(player) = shooter(host) else { return };
                let held = USE_DURATION - event.remaining;
                let now = SERVER_TICKS.load(Ordering::Relaxed);
                let uuid = player.uuid();
                let mut ticks = AIM_TICKS.lock().unwrap();
                let was = ticks.get(&uuid).map(|&(n, _)| n);
                ticks.insert(uuid, (held, now));
                // The lever's click, LEVER_TICKS into a re-armed aim: a fresh
                // aim after a shot (AIM_TICKS starts at 0) crosses the count
                // exactly when the lever delay ends.
                if was == Some(0) && held >= LEVER_TICKS {
                    drop(ticks);
                    play_sound_at(host, player, LEVER_SOUND, UI_VOLUME, LEVER_PITCH);
                }
            }
        }
    }

    /// Letting go of the aim clears both sides' aim state. The shot itself is
    /// the left click's payload; nothing fires here anymore.
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
