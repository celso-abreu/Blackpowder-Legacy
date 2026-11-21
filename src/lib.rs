//! Blackpowder Legacy — Winchester 1873 lever-action rifle (wasm mod).
//!
//! Controls (v0.4):
//! - Hold right click: shoulder and aim the rifle — a fixed zoom
//!   (`compute_fov_modifier`) and a light vignette (`gui_render_post`) apply
//!   while aiming. Aiming for at least LEVER_TICKS cycles the lever.
//! - Left click while aiming: fire one round (`key_mapping_pre` cancels the
//!   vanilla attack, so the rifle never mines or punches, and a typed payload
//!   asks the server to shoot). After each shot the lever must be cycled
//!   again (keep holding right click for LEVER_TICKS).
//! - R (`input_key`): load one cartridge per press, up to the tube's 8.
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
    ClientHost, ClientTickPreEvent, ComputeFovModifierEvent, GuiRenderPostEvent, Hooks, Host,
    HostSide, Identifier, IPayloadContext, InputKeyEvent, ItemBarEvent, ItemPart, ItemStack,
    KeyAction, KeyMappingPreEvent, LivingRef, OnUseTickEvent, ParticleKey, ParticleOptions,
    PayloadId, Registrar, RegistrationContext, ReleaseUsingEvent, ServerHost, SoundSource, Text,
    ToServer, UseDurationEvent, VarInt,
};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

const WINCHESTER: &str = "blackpowder_legacy:winchester_1873";
const CARTRIDGE: &str = "blackpowder_legacy:cartridge_44";
const BULLET: &str = "blackpowder_legacy:bullet";

const TUBE_CAPACITY: u32 = 8;
const USE_DURATION: i32 = 72000; // vanilla's "use forever" duration
const LEVER_TICKS: i32 = 8; // aim ticks (0.4 s) cycling the lever before a shot

const RELOAD_KEY: &str = "key.keyboard.r";
const ATTACK_MAPPING: &str = "key.attack";

/// The aim zoom as a FOV scale (smaller = narrower). Fixed, on purpose.
const AIM_FOV: f32 = 0.6;
/// Vignette strength: the alpha at the screen's outer edge while aiming.
const VIGNETTE_ALPHA: u32 = 0x55;

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

const SMOKE_PARTICLE: &str = "minecraft:large_smoke";
/// The ammo gauge's colour (0xRRGGBB): brass, like the cartridge cases.
const AMMO_BAR_RGB: u32 = 0xC9_A2_27;

// Rifle input actions carried by the typed payload.
const ACTION_FIRE: i32 = 1;
const ACTION_RELOAD: i32 = 2;

/// Client-side aim state: set while the rifle is in use (aimed), cleared on
/// release. Drives the zoom, the vignette and the left-click fire gate.
static AIMING: AtomicBool = AtomicBool::new(false);
/// Inputs the client hooks queued for the next client tick's payload send.
static PENDING_FIRE: AtomicBool = AtomicBool::new(false);
static PENDING_RELOAD: AtomicBool = AtomicBool::new(false);

/// Aim ticks per player (lever cycle progress), server-side. Written by
/// `on_use_tick`, cleared by `release_using` and by each shot.
static AIM_TICKS: Mutex<BTreeMap<u128, i32>> = Mutex::new(BTreeMap::new());

/// The registered serverbound channel for rifle inputs.
static RIFLE_INPUT: OnceLock<ToServer<RifleInput>> = OnceLock::new();
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
        let _ = SMOKE_KEY.set(r.particle_key(SMOKE_PARTICLE));
        let mut payloads = r.payload_handlers();
        let mut view = payloads.registrar("1");
        let input = view.play_to_server_typed::<RifleInput>(
            PayloadId("blackpowder_legacy:rifle_input".into()),
            on_rifle_input,
        );
        let _ = RIFLE_INPUT.set(input);
    }

    /// R reloads: a physical R press queues one cartridge load. The server
    /// validates (rifle in hand, room in the tube, cartridges in the
    /// inventory), so stray presses cost one harmless packet.
    fn input_key(&self, _: &mut dyn ClientHost, event: &mut InputKeyEvent<'_>) {
        if event.key == RELOAD_KEY && event.action == KeyAction::Press {
            PENDING_RELOAD.store(true, Ordering::Relaxed);
        }
    }

    /// Left click while aiming fires: cancel the vanilla attack press (no
    /// mining, no punch) and queue the shot. Outside an aim the attack
    /// behaves as usual, so carrying the rifle never blocks the pickaxe use.
    fn key_mapping_pre(&self, _: &mut dyn ClientHost, event: &mut KeyMappingPreEvent<'_>) {
        if event.mapping == ATTACK_MAPPING
            && event.pressed
            && AIMING.load(Ordering::Relaxed)
        {
            event.canceled = true;
            PENDING_FIRE.store(true, Ordering::Relaxed);
        }
    }

    /// Send the queued rifle inputs on the client tick, the moment a mutable
    /// client host is available (payload sends need one).
    fn client_tick_pre(&self, host: &mut dyn ClientHost, _: &mut ClientTickPreEvent<'_>) {
        let Some(input) = RIFLE_INPUT.get() else { return };
        if PENDING_FIRE.swap(false, Ordering::Relaxed) {
            let _ = input.send(
                host,
                &RifleInput {
                    action: VarInt(ACTION_FIRE),
                },
            );
        }
        if PENDING_RELOAD.swap(false, Ordering::Relaxed) {
            let _ = input.send(
                host,
                &RifleInput {
                    action: VarInt(ACTION_RELOAD),
                },
            );
        }
    }

    /// The aim zoom: a fixed FOV scale while the rifle is shouldered.
    fn compute_fov_modifier(&self, _: &dyn ClientHost, event: &mut ComputeFovModifierEvent<'_>) {
        if AIMING.load(Ordering::Relaxed) {
            event.modifier = AIM_FOV;
        }
    }

    /// A light vignette while aiming: soft dark bands fading in from the
    /// screen edges. Hidden with the F1 HUD. `batch` is the GUI's
    /// `DrawContext`; colours are ARGB.
    fn gui_render_post(&self, _: &dyn ClientHost, event: &mut GuiRenderPostEvent<'_>) {
        if !AIMING.load(Ordering::Relaxed) || event.gui_hidden {
            return;
        }
        let [w, h] = event.batch.size();
        let band = h * 0.16;
        let side = w * 0.10;
        let solid = VIGNETTE_ALPHA << 24; // opaque at the edge
        let clear = 0u32; // transparent toward the centre
        let batch = &mut *event.batch;
        // Top and bottom fade down/up; left and right are drawn sideways by
        // rotating the gradient's rect (a gradient runs along its height).
        batch.fill_gradient([0.0, 0.0, w, band], solid, clear);
        batch.fill_gradient([0.0, h - band, w, band], clear, solid);
        batch.fill_gradient([0.0, 0.0, side, h], solid, clear);
        batch.fill_gradient([w - side, 0.0, side, h], clear, solid);
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

/// Load one cartridge into the tube (one R press), with the loading sounds
/// and the ammo counter. Server-side.
fn reload_one(host: &mut dyn ServerHost, player: mod_api::PlayerRef) {
    let Some((slot, rifle)) = find_rifle(host, player) else {
        return;
    };
    let loaded = rounds(&rifle);
    if loaded >= TUBE_CAPACITY || !take_cartridge(host, player) {
        return;
    }
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

/// Fire one round, if the lever was cycled (aimed LEVER_TICKS) and the tube
/// is loaded. Cycling starts over after each shot. Server-side.
fn fire_rifle(host: &mut dyn ServerHost, player: mod_api::PlayerRef, smoke: ParticleKey) {
    let uuid = player.uuid();
    let aimed = AIM_TICKS.lock().unwrap().get(&uuid).copied().unwrap_or(0);
    if aimed < LEVER_TICKS {
        return; // the lever is still cycling
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
    AIM_TICKS.lock().unwrap().insert(uuid, 0);
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
    /// the server counts lever ticks for the next shot.
    fn on_use_tick(&self, host: HostSide<'_>, event: &mut OnUseTickEvent<'_>) {
        match host {
            HostSide::Client(_) => AIMING.store(true, Ordering::Relaxed),
            HostSide::Server(host) => {
                let Some(player) = shooter(host) else { return };
                let held = USE_DURATION - event.remaining;
                AIM_TICKS.lock().unwrap().insert(player.uuid(), held);
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
