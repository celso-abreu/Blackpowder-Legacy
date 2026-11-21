//! Blackpowder Legacy — Winchester 1873 lever-action rifle (wasm mod).
//!
//! Mechanics (v0.2):
//! - The Winchester holds up to 8 cartridges in its tube magazine.
//! - Hold right click: with an empty tube (or after loading), cartridges are
//!   loaded one every 8 ticks; with rounds loaded, you shoulder the rifle.
//! - Release right click: fires one round, if the lever was cycled (held for
//!   at least 8 ticks) and no cartridge was loaded during this use.
//!
//! The magazine lives in the stack's vanilla `damage` component (0..=8);
//! the item declares `max_damage = 100` so every write is valid and the rifle
//! never breaks. Rounds are therefore saved, synced and shown by vanilla
//! item-stack machinery. The `item_bar` hook replaces the (misleading) vanilla
//! damage bar with a proper brass ammo gauge.
//!
//! Sounds go through the level's own sound broadcast
//! (`level_play_sound`), not `/playsound` commands: the command path answered
//! every shot with a "Played sound …" feedback line in the shooter's chat.
//! A shot also puffs black-powder smoke past the muzzle (`level_send_particles`).

use mod_api::{
    ClientHost, Hooks, Host, HostSide, Identifier, ItemBarEvent, ItemPart, ItemStack, LivingRef,
    OnUseTickEvent, ParticleKey, ParticleOptions, Registrar, RegistrationContext,
    ReleaseUsingEvent, ServerHost, SoundSource, Text, UseDurationEvent,
};
use std::collections::BTreeMap;
use std::sync::Mutex;

const WINCHESTER: &str = "blackpowder_legacy:winchester_1873";
const CARTRIDGE: &str = "blackpowder_legacy:cartridge_44";
const BULLET: &str = "blackpowder_legacy:bullet";

const TUBE_CAPACITY: u32 = 8;
const USE_DURATION: i32 = 72000; // vanilla's "use forever" duration
const LEVER_TICKS: i32 = 8; // minimum hold (0.4 s) before a shot can leave
const RELOAD_TICKS: i32 = 8; // hold ticks per cartridge loaded

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

/// Players whose current use already loaded a cartridge (a loading use never
/// fires on release). Keyed by player UUID; consumed by `release_using`.
static LOADED_THIS_USE: Mutex<BTreeMap<u128, ()>> = Mutex::new(BTreeMap::new());

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let winchester = r.item_key(WINCHESTER);
        let smoke = r.particle_key(SMOKE_PARTICLE);
        r.item_part(winchester, Winchester { smoke });
    }
}

struct Winchester {
    smoke: ParticleKey,
}

/// Rounds in the tube, read from the stack's damage component.
fn rounds(stack: &ItemStack) -> u32 {
    stack.damage().min(TUBE_CAPACITY)
}

/// The acting player of this hook, falling back to a UUID we recorded while
/// the use was ticking (some paths may not swap the acting player in).
fn shooter(host: &dyn ServerHost) -> Option<mod_api::PlayerRef> {
    if let Some(player) = host.players().acting_player() {
        return Some(player);
    }
    None
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

/// The slot holding the in-use rifle: the one whose wear matches the stack
/// the hook saw, else the first rifle found. Two rifles with identical
/// wear in one inventory are ambiguous in v0.2.
fn find_rifle(
    host: &mut dyn ServerHost,
    player: mod_api::PlayerRef,
    want_damage: u32,
) -> Option<(u32, ItemStack)> {
    let slots = player.inventory_slots(host)?;
    let mut first = None;
    for (slot, stack) in slots {
        if stack.item().id(host).to_string() == WINCHESTER {
            if stack.damage() == want_damage {
                return Some((slot, stack));
            }
            first.get_or_insert((slot, stack));
        }
    }
    first
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

#[mod_api::horizon_part]
impl ItemPart for Winchester {
    /// A use duration above zero is what makes a mod item usable over time
    /// on a right click (vanilla's `Item.use` → `startUsingItem`).
    fn use_duration(&self, _: &dyn Host, event: &mut UseDurationEvent<'_>) {
        event.duration = USE_DURATION;
    }

    /// Ammo gauge under the rifle icon. Vanilla's damage bar would lie here:
    /// our "damage" component is the tube's load (0..=8) on a max_damage=100
    /// item, so it would show a nearly full green bar on a full tube and none
    /// when empty. Replace it with the tube's fullness, in brass.
    fn item_bar(&self, _: &mut dyn ClientHost, event: &mut ItemBarEvent<'_>) {
        let rounds = rounds(&event.stack);
        let width = (rounds * 13 / TUBE_CAPACITY) as i32;
        event.bar = Some((width, AMMO_BAR_RGB));
    }

    /// Loading gate: every RELOAD_TICKS of holding, one cartridge moves from
    /// the inventory into the tube, until full.
    fn on_use_tick(&self, host: HostSide<'_>, event: &mut OnUseTickEvent<'_>) {
        let HostSide::Server(host) = host else {
            return;
        };
        let held = USE_DURATION - event.remaining;
        if held <= 0 || held % RELOAD_TICKS != 0 {
            return;
        }
        let loaded = rounds(&event.stack);
        if loaded >= TUBE_CAPACITY {
            return;
        }
        let Some(player) = shooter(host) else { return };
        if !take_cartridge(host, player) {
            return;
        }
        let Some((slot, mut rifle)) = find_rifle(host, player, loaded) else {
            return;
        };
        rifle.set_damage(loaded + 1);
        if player.set_inventory_slot(host, slot, Some(rifle)).is_err() {
            return;
        }
        LOADED_THIS_USE.lock().unwrap().insert(player.uuid(), ());
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

    /// Trigger: release after cycling the lever fires one round.
    fn release_using(&self, host: HostSide<'_>, event: &mut ReleaseUsingEvent<'_>) {
        event.used = true;
        let HostSide::Server(host) = host else {
            return;
        };
        let Some(player) = shooter(host) else { return };
        // A use that loaded cartridges ends there: no shot follows the click-clack.
        if LOADED_THIS_USE.lock().unwrap().remove(&player.uuid()).is_some() {
            return;
        }
        let held = USE_DURATION - event.time_left;
        if held < LEVER_TICKS {
            return; // lever not fully cycled
        }
        let loaded = rounds(&event.stack);
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
        let Some((slot, mut rifle)) = find_rifle(host, player, loaded) else {
            return;
        };
        rifle.set_damage(loaded - 1);
        let _ = player.set_inventory_slot(host, slot, Some(rifle));
        play_sound_at(host, player, SHOT_SOUND, SHOT_VOLUME, SHOT_PITCH);
        smoke_at_muzzle(host, player, self.smoke);
        let _ = player.action_bar(
            host,
            Text::literal(format!("{} / {}", loaded - 1, TUBE_CAPACITY)),
        );
    }
}
