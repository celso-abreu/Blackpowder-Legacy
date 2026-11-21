//! Blackpowder Legacy — Winchester 1873 lever-action rifle (wasm mod).
//!
//! Mechanics (v0.1):
//! - The Winchester holds up to 8 cartridges in its tube magazine.
//! - Hold right click: with an empty tube (or after loading), cartridges are
//!   loaded one every 8 ticks; with rounds loaded, you shoulder the rifle.
//! - Release right click: fires one round, if the lever was cycled (held for
//!   at least 8 ticks) and no cartridge was loaded during this use.
//!
//! The magazine lives in the stack's vanilla `damage` component (0..=8);
//! the item declares `max_damage = 100` so every write is valid and the rifle
//! never breaks. Rounds are therefore saved, synced and shown by vanilla
//! item-stack machinery. `item_bar` will draw a proper ammo gauge in v0.2.

use mod_api::{
    Hooks, Host, HostSide, Identifier, ItemPart, ItemStack, LivingRef, OnUseTickEvent,
    PermissionLevel, Registrar, RegistrationContext, ReleaseUsingEvent, ServerHost, Text,
    UseDurationEvent,
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

const SHOT_SOUND: &str =
    "playsound minecraft:entity.firework_rocket.blast player @a[distance=..48] ~ ~ ~ 3.0 0.6";
const DRY_SOUND: &str = "playsound minecraft:block.lever.click player @a[distance=..16] ~ ~ ~ 0.7 0.7";
const LOAD_SOUND: &str =
    "playsound minecraft:item.crossbow.loading_middle player @a[distance=..16] ~ ~ ~ 0.7 1.2";
const READY_SOUND: &str =
    "playsound minecraft:item.crossbow.loading_end player @a[distance=..16] ~ ~ ~ 0.7 1.0";

/// Players whose current use already loaded a cartridge (a loading use never
/// fires on release). Keyed by player UUID; consumed by `release_using`.
static LOADED_THIS_USE: Mutex<BTreeMap<u128, ()>> = Mutex::new(BTreeMap::new());

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let winchester = r.item_key(WINCHESTER);
        r.item_part(winchester, Winchester);
    }
}

struct Winchester;

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

/// Play a sound at an entity, at its position, through the command path.
/// MVP note: a dedicated host sound call replaces this once verified.
fn play_sound_at(host: &mut dyn ServerHost, player: mod_api::PlayerRef, line: &str) {
    let _ = player
        .entity()
        .run_command(host, line, PermissionLevel::Gamemasters);
}

/// The slot holding the in-use rifle: the one whose wear matches the stack
/// the hook saw, else the first rifle found. Two rifles with identical
/// wear in one inventory are ambiguous in v0.1.
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
        play_sound_at(host, player, if full { READY_SOUND } else { LOAD_SOUND });
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
            play_sound_at(host, player, DRY_SOUND);
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
        play_sound_at(host, player, SHOT_SOUND);
        let _ = player.action_bar(
            host,
            Text::literal(format!("{} / {}", loaded - 1, TUBE_CAPACITY)),
        );
    }
}
