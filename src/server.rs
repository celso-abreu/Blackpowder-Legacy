//! Server-authoritative `.44-40` firing, stance spread, muzzle VFX, per-shell
//! side-gate reload loop, network input/skin handlers, and `ItemPart` hooks.

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use mod_api::{
    ClientHost, EntityUuid, Hand, Host, HostSide, IPayloadContext, Identifier, InteractionResult,
    ItemBarEvent, ItemPart, ItemStack, LivingRef, OnUseTickEvent, ParticleKey, ParticleOptions,
    ReleaseUsingEvent, ServerHost, ShouldCauseReequipAnimationEvent, SoundSource, UseDurationEvent,
    UseItemEvent, VarInt,
};

use crate::constants::*;
use crate::skin::{default_skin_for_uuid, parse_mojang_skin_property};
use crate::{
    RifleInput, RifleSkinSync, RifleSync, AIMING, AIM_LAST_TICK, CLIENT_RELOADING,
    CLIENT_SPRINTING, CLIENT_TICKS, FLAME_KEY, RIFLE_SKIN_SYNC, RIFLE_SYNC, SERVER_TICKS,
    SMOKE_KEY, WHITE_SMOKE_KEY,
};

static AIM_TICKS: Mutex<BTreeMap<u128, (i32, u64)>> = Mutex::new(BTreeMap::new());
static LEVER_READY: Mutex<BTreeMap<u128, u64>> = Mutex::new(BTreeMap::new());
static RELOAD_SESSIONS: Mutex<BTreeMap<u128, ReloadSession>> = Mutex::new(BTreeMap::new());

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

pub(crate) struct Winchester;

pub(crate) fn rounds(stack: &ItemStack) -> u32 {
    stack.damage().min(TUBE_CAPACITY)
}

fn shooter(host: &dyn ServerHost) -> Option<mod_api::PlayerRef> {
    host.players().acting_player()
}

fn is_player_reloading(uuid: u128) -> bool {
    RELOAD_SESSIONS.lock().unwrap().contains_key(&uuid)
}

fn cancel_reload_session(host: &mut dyn ServerHost, player: mod_api::PlayerRef) -> bool {
    let removed = RELOAD_SESSIONS
        .lock()
        .unwrap()
        .remove(&player.uuid())
        .is_some();
    if removed {
        let cur = held_rifle(host, player).map_or(0, |(_, r)| rounds(&r));
        send_sync(host, player, SYNC_RELOAD_ABORTED, cur);
    }
    removed
}

pub(crate) fn on_rifle_input(
    value: RifleInput,
    cx: &mut dyn IPayloadContext,
) -> Result<(), String> {
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

    let lever_ready = LEVER_READY
        .lock()
        .unwrap()
        .get(&uuid)
        .copied()
        .unwrap_or(0);
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

pub(crate) fn tick_reload_sessions(host: &mut dyn ServerHost, now: u64) {
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
                        let empty_chamber =
                            session.from_empty && session.inserted_any && loaded > 0;
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
                    LEVER_READY
                        .lock()
                        .unwrap()
                        .insert(uuid, now + if empty_chamber { 2 } else { 1 });
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
