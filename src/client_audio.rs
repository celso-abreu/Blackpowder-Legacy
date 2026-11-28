//! Client-side audio mixer dispatch, tracked action foley (`SoundRef` cancellation/fading),
//! concussive indoor blast muffling, and server-to-client sync payload handlers.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use mod_api::{ClientHost, IPayloadContext, LightLayer, SoundPlay, SoundRef, SoundSource};

use crate::arms::invalidate_skin_sprite_cache;
use crate::constants::*;
use crate::exploration::distance_3d;
use crate::pose::ActionAnim;
use crate::server::unpack_coord_sub16;
use crate::settings::{
    client_settings, default_channel_for_local_sound, mix_client_sound, SoundChannel,
};
use crate::skin::ResolvedSkin;
use crate::world_fx::spawn_spent_casing_from_remote_actor;
use crate::{
    client_anim, RifleAcousticSync, RifleSkinSync, RifleSync, RifleThirdPersonSync, AIMING,
    CLIENT_RELOADING, CLIENT_TICKS, HEADSHOT_HITMARKER, HITMARKER_TIMER_BITS, LOCAL_ENTITY_ID,
    PENDING_AUTO_RELOAD, TOGGLE_AIM_LATCH,
};

#[derive(Clone, Copy)]
pub(crate) struct RemoteTpState {
    pub(crate) state: i32,
    pub(crate) set_tick: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActionSoundTag {
    Draw,
    Inspect,
    Reload,
}

pub(crate) static REMOTE_TP_STATES: Mutex<BTreeMap<u32, RemoteTpState>> =
    Mutex::new(BTreeMap::new());
static ACTIVE_ACTION_SOUND: Mutex<Option<(ActionSoundTag, SoundRef)>> = Mutex::new(None);
static REMOTE_ACTION_SOUNDS: Mutex<BTreeMap<u32, SoundRef>> = Mutex::new(BTreeMap::new());
static PENDING_CONCUSSION: Mutex<Option<(f32, f32, u32)>> = Mutex::new(None);
static CONCUSSION_ACTIVE: AtomicBool = AtomicBool::new(false);

const CONCUSSION_EXEMPT_SOUNDS: &[&str] = &[
    SND_FIRE,
    SND_THUMP,
    SND_ECHO_CAVE,
    SND_ECHO_OPEN,
    SND_LEVER,
    SND_COCK,
    SND_BULLET_CRACK,
    SND_BULLET_WHIZ,
];

fn stop_or_fade_sound(host: &dyn ClientHost, sound: SoundRef, fade_ticks: u32) {
    if fade_ticks == 0 {
        let _ = sound.stop(host);
    } else {
        let _ = sound.fade(host, 0.0, fade_ticks);
    }
}

pub(crate) fn cancel_action_sound(
    host: &dyn ClientHost,
    filter: Option<ActionSoundTag>,
    fade_ticks: u32,
) {
    let prev = {
        let mut slot = ACTIVE_ACTION_SOUND.lock().unwrap();
        match *slot {
            Some((tag, sref)) if filter.is_none_or(|want| want == tag) => {
                *slot = None;
                Some(sref)
            }
            _ => None,
        }
    };
    if let Some(sref) = prev {
        stop_or_fade_sound(host, sref, fade_ticks);
    }
}

pub(crate) fn clear_all_tracked_sounds(host: &dyn ClientHost) {
    cancel_action_sound(host, None, 0);
    let remotes: Vec<SoundRef> = core::mem::take(&mut *REMOTE_ACTION_SOUNDS.lock().unwrap())
        .into_values()
        .collect();
    for sref in remotes {
        let _ = sref.stop(host);
    }
}

pub(crate) fn reset_concussion_state(host: &mut dyn ClientHost) {
    *PENDING_CONCUSSION.lock().unwrap() = None;
    if CONCUSSION_ACTIVE.swap(false, Ordering::Relaxed) {
        let _ = host.muffle_sounds(0.0, 0.0, 0, &[]);
    }
}

pub(crate) fn flush_pending_concussion(host: &mut dyn ClientHost, allow_concussion: bool) {
    if !allow_concussion {
        reset_concussion_state(host);
    } else if let Some((amount, lowpass, muffle_ticks)) = PENDING_CONCUSSION.lock().unwrap().take()
        && host.muffle_sounds(amount, lowpass, muffle_ticks, CONCUSSION_EXEMPT_SOUNDS)
    {
        CONCUSSION_ACTIVE.store(true, Ordering::Relaxed);
    }
}

/// Evicts stale 3rd-person pose entries and stops remote reload `SoundRef`s whose actors have
/// despawned or left entity tracking range.
pub(crate) fn prune_remote_client_tables(host: &dyn ClientHost, now: u64) {
    REMOTE_TP_STATES
        .lock()
        .unwrap()
        .retain(|_, v| now.saturating_sub(v.set_tick) <= 120);

    let dead_remotes: Vec<SoundRef> = {
        let mut map = REMOTE_ACTION_SOUNDS.lock().unwrap();
        let stale_ids: Vec<u32> = map
            .keys()
            .copied()
            .filter(|&eid| host.entity(eid).is_none())
            .collect();
        stale_ids
            .into_iter()
            .filter_map(|eid| map.remove(&eid))
            .collect()
    };
    for sref in dead_remotes {
        let _ = sref.stop(host);
    }
}

pub(crate) fn client_pitch_jitter(salt: u64) -> f32 {
    let mut h = CLIENT_TICKS
        .load(Ordering::Relaxed)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(salt.wrapping_mul(0xBF58_476D_1CE4_E5B9));
    h ^= h >> 30;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 27;
    (((h & 0xFFFF) as f32) * (1.0 / 32_767.5) - 1.0).clamp(-1.0, 1.0)
}

pub(crate) fn play_local_foley_channel(
    host: &dyn ClientHost,
    channel: SoundChannel,
    event_id: &str,
    volume: f32,
    pitch: f32,
) -> Option<SoundRef> {
    let cfg = client_settings();
    let (vol, pit) = mix_client_sound(&cfg, channel, volume, pitch)?;
    let mut sound = SoundPlay::ui(event_id).with_volume(vol).with_pitch(pit);
    sound.source = SoundSource::Players;
    host.play_local_sound(&sound, 0)
}

pub(crate) fn play_local_foley(
    host: &dyn ClientHost,
    event_id: &str,
    volume: f32,
    pitch: f32,
) -> Option<SoundRef> {
    play_local_foley_channel(
        host,
        default_channel_for_local_sound(event_id),
        event_id,
        volume,
        pitch,
    )
}

pub(crate) fn play_tracked_action_sound(
    host: &dyn ClientHost,
    tag: ActionSoundTag,
    channel: SoundChannel,
    event_id: &str,
    volume: f32,
    pitch: f32,
) {
    cancel_action_sound(host, None, 2);
    if let Some(sref) = play_local_foley_channel(host, channel, event_id, volume, pitch) {
        *ACTIVE_ACTION_SOUND.lock().unwrap() = Some((tag, sref));
    }
}

pub(crate) fn play_local_positional_channel(
    host: &dyn ClientHost,
    channel: SoundChannel,
    event_id: &str,
    position: [f64; 3],
    volume: f32,
    pitch: f32,
    after_ticks: u32,
) -> Option<SoundRef> {
    let cfg = client_settings();
    let (vol, pit) = mix_client_sound(&cfg, channel, volume, pitch)?;
    let sound = SoundPlay::at(event_id, SoundSource::Players, position)
        .with_volume(vol)
        .with_pitch(pit);
    host.play_local_sound(&sound, after_ticks)
}

pub(crate) fn play_local_positional_foley(
    host: &dyn ClientHost,
    event_id: &str,
    position: [f64; 3],
    volume: f32,
    pitch: f32,
    after_ticks: u32,
) -> Option<SoundRef> {
    play_local_positional_channel(
        host,
        default_channel_for_local_sound(event_id),
        event_id,
        position,
        volume,
        pitch,
        after_ticks,
    )
}

pub(crate) fn on_rifle_sync(value: RifleSync, cx: &mut dyn IPayloadContext) -> Result<(), String> {
    let cfg = client_settings();
    if value.kind.0 == SYNC_HIT_CONFIRM || value.kind.0 == SYNC_HEADSHOT_CONFIRM {
        let is_headshot = value.kind.0 == SYNC_HEADSHOT_CONFIRM;
        HEADSHOT_HITMARKER.store(is_headshot, Ordering::Relaxed);
        HITMARKER_TIMER_BITS.store(
            (if is_headshot { 0.22_f32 } else { 0.14_f32 }).to_bits(),
            Ordering::Relaxed,
        );
        if let Some((snd, vol, pitch)) = cfg.hit_confirm_sound.sound_spec()
            && let Ok(host) = cx.client_host()
        {
            if is_headshot {
                let _ = play_local_foley_channel(
                    host,
                    SoundChannel::Feedback,
                    snd,
                    (vol * 1.18).min(1.0),
                    (pitch * 1.18).min(2.0),
                );
                let _ = play_local_foley_channel(
                    host,
                    SoundChannel::Feedback,
                    SND_HIT_CONFIRM_CHIME,
                    (vol * 0.85).min(0.85),
                    1.82,
                );
            } else {
                let _ = play_local_foley_channel(host, SoundChannel::Feedback, snd, vol, pitch);
            }
        }
        return Ok(());
    }

    let rounds_now = (value.rounds.0.max(0) as u32).min(TUBE_CAPACITY);
    let mut tracked_foley: Option<(ActionSoundTag, SoundChannel, &'static str, f32, f32)> = None;
    let mut oneshot_foley: Option<(SoundChannel, &'static str, f32, f32)> = None;
    let mut stop_tracked = false;

    {
        let mut anim = client_anim().lock().unwrap();
        anim.known_rounds = rounds_now;
        let j = client_pitch_jitter(u64::from(rounds_now) + (value.kind.0 as u64) * 31);
        match value.kind.0 {
            SYNC_SHOT => {
                CLIENT_RELOADING.store(false, Ordering::Relaxed);
                stop_tracked = true;
                anim.on_shot(rounds_now);
            }
            SYNC_DRY_CLICK => {
                stop_tracked = true;
                anim.on_dry_click(rounds_now);
                if cfg.dry_click_alert {
                    oneshot_foley =
                        Some((SoundChannel::Feedback, SND_DRY_FIRE, 0.74, 1.0 + 0.03 * j));
                }
                if rounds_now == 0 && cfg.auto_reload {
                    PENDING_AUTO_RELOAD.store(true, Ordering::Relaxed);
                }
            }
            SYNC_WET_MISFIRE => {
                // World-space Mono `SND_MISFIRE_WET` is already emitted at the flooded breech on
                // the server (`emit_wet_misfire`); only stop interrupted foley & animate trigger here.
                CLIENT_RELOADING.store(false, Ordering::Relaxed);
                stop_tracked = true;
                anim.on_dry_click(rounds_now);
            }
            SYNC_RELOAD_START => {
                CLIENT_RELOADING.store(true, Ordering::Relaxed);
                AIMING.store(false, Ordering::Relaxed);
                TOGGLE_AIM_LATCH.store(false, Ordering::Relaxed);
                anim.action = ActionAnim::ReloadIntro { elapsed: 0.0 };
                tracked_foley = Some((
                    ActionSoundTag::Reload,
                    SoundChannel::Reload,
                    SND_RELOAD_INTRO,
                    0.90,
                    1.0 + 0.025 * j,
                ));
            }
            SYNC_SHELL_INSERTED => {
                CLIENT_RELOADING.store(true, Ordering::Relaxed);
                anim.action = ActionAnim::ReloadInsert { elapsed: 0.0 };
                tracked_foley = Some((
                    ActionSoundTag::Reload,
                    SoundChannel::Reload,
                    SND_RELOAD_INSERT,
                    0.92,
                    1.0 + 0.03 * j,
                ));
            }
            SYNC_RELOAD_END_NORM => {
                CLIENT_RELOADING.store(false, Ordering::Relaxed);
                anim.action = ActionAnim::ReloadEndNorm { elapsed: 0.0 };
                tracked_foley = Some((
                    ActionSoundTag::Reload,
                    SoundChannel::Reload,
                    SND_RELOAD_END,
                    0.88,
                    1.0 + 0.02 * j,
                ));
            }
            SYNC_RELOAD_END_EMPTY => {
                // Empty-chamber close cycles the lever at `t = 0.15s` + cocks the hammer at `t = 0.40s`
                // (`Mechanism` channel); fade any tail from the last shell insert instead of overlapping
                // `SND_RELOAD_END` on top of the lever throw.
                CLIENT_RELOADING.store(false, Ordering::Relaxed);
                stop_tracked = true;
                anim.action = ActionAnim::ReloadEndEmpty {
                    elapsed: 0.0,
                    lever_sound_played: false,
                    cock_sound_played: false,
                };
            }
            SYNC_RELOAD_ABORTED => {
                CLIENT_RELOADING.store(false, Ordering::Relaxed);
                stop_tracked = true;
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

    if let Ok(host) = cx.client_host() {
        if stop_tracked {
            cancel_action_sound(host, None, 2);
        }
        if let Some((tag, ch, snd, vol, pitch)) = tracked_foley {
            play_tracked_action_sound(host, tag, ch, snd, vol, pitch);
        }
        if let Some((ch, snd, vol, pitch)) = oneshot_foley {
            let _ = play_local_foley_channel(host, ch, snd, vol, pitch);
        }
    }
    Ok(())
}

pub(crate) fn on_rifle_skin_sync(
    value: RifleSkinSync,
    _: &mut dyn IPayloadContext,
) -> Result<(), String> {
    invalidate_skin_sprite_cache();
    let mut anim = client_anim().lock().unwrap();
    anim.skin = ResolvedSkin {
        primary: value.sprite,
        fallback: value.fallback,
        slim: value.slim.0 != 0,
    };
    Ok(())
}

fn resolve_remote_breech_pos(host: &dyn ClientHost, eid: u32) -> Option<[f64; 3]> {
    let ent = host.entity(eid)?;
    if let Some(eye) = ent.eye_position(host) {
        Some([eye[0], eye[1] - 0.24, eye[2]])
    } else {
        ent.position(host).map(|p| [p[0], p[1] + 1.32, p[2]])
    }
}

fn replace_remote_action_sound(host: &dyn ClientHost, eid: u32, next: Option<SoundRef>) {
    let prev = {
        let mut map = REMOTE_ACTION_SOUNDS.lock().unwrap();
        if map.len() >= 64 && !map.contains_key(&eid) {
            map.clear();
        }
        match next {
            Some(sref) => map.insert(eid, sref),
            None => map.remove(&eid),
        }
    };
    if let Some(old) = prev {
        stop_or_fade_sound(host, old, 2);
    }
}

pub(crate) fn on_rifle_tp_sync(
    value: RifleThirdPersonSync,
    cx: &mut dyn IPayloadContext,
) -> Result<(), String> {
    if value.entity_id.0 <= 0 {
        return Ok(());
    }
    let eid = value.entity_id.0 as u32;
    let now = CLIENT_TICKS.load(Ordering::Relaxed);
    let cue = value.state.0;

    // 1. Update 3rd-person remote pose table (`TP_CUE_DRY_CLICK` is audio-only, preserving pose).
    if cue != TP_CUE_DRY_CLICK {
        let mut map = REMOTE_TP_STATES.lock().unwrap();
        match cue {
            TP_STATE_IDLE | TP_CUE_RELOAD_END_NORM | TP_CUE_LAST_SHOT_EJECT => {
                map.remove(&eid);
            }
            TP_STATE_RELOADING | TP_CUE_RELOAD_INSERT => {
                if map.len() >= 64 {
                    map.retain(|_, v| now.saturating_sub(v.set_tick) <= 120);
                }
                map.insert(
                    eid,
                    RemoteTpState {
                        state: TP_STATE_RELOADING,
                        set_tick: now,
                    },
                );
            }
            TP_STATE_LEVER_CYCLE | TP_CUE_EMPTY_CHAMBER_LEVER => {
                if map.len() >= 64 {
                    map.retain(|_, v| now.saturating_sub(v.set_tick) <= 120);
                }
                map.insert(
                    eid,
                    RemoteTpState {
                        state: TP_STATE_LEVER_CYCLE,
                        set_tick: now,
                    },
                );
            }
            _ => {}
        }
    }

    // 2. Emit 3D Mono positional foley & spent brass casings for nearby remote players (`eid != local_eid`).
    if eid == LOCAL_ENTITY_ID.load(Ordering::Relaxed) {
        return Ok(());
    }
    let Ok(host) = cx.client_host() else {
        return Ok(());
    };
    if cue == TP_STATE_IDLE {
        replace_remote_action_sound(host, eid, None);
        return Ok(());
    }
    let Some(pos) = resolve_remote_breech_pos(host, eid) else {
        return Ok(());
    };
    let j = client_pitch_jitter(u64::from(eid).wrapping_mul(17) ^ (cue as u64));
    match cue {
        TP_STATE_RELOADING => {
            let sref = play_local_positional_channel(
                host,
                SoundChannel::Reload,
                SND_RELOAD_INTRO,
                pos,
                0.78,
                1.0 + 0.025 * j,
                0,
            );
            replace_remote_action_sound(host, eid, sref);
        }
        TP_CUE_RELOAD_INSERT => {
            let sref = play_local_positional_channel(
                host,
                SoundChannel::Reload,
                SND_RELOAD_INSERT,
                pos,
                0.82,
                1.0 + 0.03 * j,
                0,
            );
            replace_remote_action_sound(host, eid, sref);
        }
        TP_CUE_RELOAD_END_NORM => {
            let sref = play_local_positional_channel(
                host,
                SoundChannel::Reload,
                SND_RELOAD_END,
                pos,
                0.76,
                1.0 + 0.02 * j,
                0,
            );
            replace_remote_action_sound(host, eid, sref);
        }
        TP_STATE_LEVER_CYCLE => {
            replace_remote_action_sound(host, eid, None);
            spawn_spent_casing_from_remote_actor(host, eid);
            let _ = play_local_positional_channel(
                host,
                SoundChannel::Mechanism,
                SND_LEVER,
                pos,
                0.80,
                1.0 + 0.025 * j,
                3,
            );
            let _ = play_local_positional_channel(
                host,
                SoundChannel::Mechanism,
                SND_COCK,
                pos,
                0.66,
                1.02 + 0.025 * j,
                8,
            );
        }
        TP_CUE_LAST_SHOT_EJECT => {
            replace_remote_action_sound(host, eid, None);
            spawn_spent_casing_from_remote_actor(host, eid);
        }
        TP_CUE_EMPTY_CHAMBER_LEVER => {
            replace_remote_action_sound(host, eid, None);
            let _ = play_local_positional_channel(
                host,
                SoundChannel::Mechanism,
                SND_LEVER,
                pos,
                0.80,
                1.0 + 0.025 * j,
                3,
            );
            let _ = play_local_positional_channel(
                host,
                SoundChannel::Mechanism,
                SND_COCK,
                pos,
                0.66,
                1.02 + 0.025 * j,
                8,
            );
        }
        TP_CUE_DRY_CLICK => {
            let _ = play_local_positional_channel(
                host,
                SoundChannel::Mechanism,
                SND_DRY_FIRE,
                pos,
                0.72,
                1.0 + 0.03 * j,
                0,
            );
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn on_rifle_acoustic_sync(
    value: RifleAcousticSync,
    cx: &mut dyn IPayloadContext,
) -> Result<(), String> {
    let Ok(host) = cx.client_host() else {
        return Ok(());
    };
    let cfg = client_settings();
    let pos = [
        unpack_coord_sub16(value.x_sub16),
        unpack_coord_sub16(value.y_sub16),
        unpack_coord_sub16(value.z_sub16),
    ];
    let intensity = (value.intensity_pm.0 as f32 * 0.001).clamp(0.0, 1.0);

    let cam = host.camera();
    let listener_pos = cam
        .as_ref()
        .map(|c| c.position)
        .or_else(|| host.local_player().and_then(|p| p.entity().eye_position(host)));
    let dist = listener_pos.map_or(0.0, |lp| distance_3d(pos, lp));
    // Speed of sound (~340 m/s = 17 blocks/tick at 20 Hz) for realistic downrange report arrival.
    let prop_ticks = (dist / 17.0).floor().clamp(0.0, 12.0) as u32;
    let salt = (value.x_sub16.0 as u64)
        .wrapping_mul(31)
        .wrapping_add(value.z_sub16.0 as u64);
    let j = client_pitch_jitter(salt);

    match value.cue.0 {
        ACOUSTIC_CUE_BULLET_CRACK => {
            let vol = 0.72 + 0.28 * intensity;
            let pitch = 0.96 + 0.12 * intensity + 0.03 * j;
            let _ = play_local_positional_channel(
                host,
                SoundChannel::WorldImpact,
                SND_BULLET_CRACK,
                pos,
                vol,
                pitch,
                0,
            );
            // Very close supersonic graze (`<= ~3m`) trails a subtle aerodynamic slipstream wake.
            if intensity >= 0.45 {
                let _ = play_local_positional_channel(
                    host,
                    SoundChannel::WorldImpact,
                    SND_BULLET_WHIZ,
                    pos,
                    0.36 * intensity,
                    1.06 + 0.03 * j,
                    1,
                );
            }
        }
        ACOUSTIC_CUE_BULLET_WHIZ => {
            let vol = 0.42 + 0.50 * intensity;
            let pitch = 0.90 + 0.18 * intensity + 0.03 * j;
            let _ = play_local_positional_channel(
                host,
                SoundChannel::WorldImpact,
                SND_BULLET_WHIZ,
                pos,
                vol,
                pitch,
                0,
            );
        }
        ACOUSTIC_CUE_SHOT_CONFINED => {
            let (vol, pitch) = if dist <= 32.0 {
                (
                    0.62 + 0.42 * intensity,
                    0.96 + 0.06 * (1.0 - intensity) + 0.025 * j,
                )
            } else {
                let fade = (1.0 - ((dist - 32.0) / 68.0) as f32).clamp(0.18, 1.0);
                (1.55 * fade, 0.88 + 0.025 * j)
            };
            let _ = play_local_positional_channel(
                host,
                SoundChannel::Echo,
                SND_ECHO_CAVE,
                pos,
                vol,
                pitch,
                (1 + prop_ticks).min(14),
            );

            // Subtle indoor/cave blast pressure concussion when detonating right near the listener.
            if cfg.blast_concussion
                && cfg.vol_master > 0.01
                && cfg.vol_fire > 0.01
                && dist <= 5.0
                && let Some(lp) = listener_pos
            {
                let dim = cam
                    .as_ref()
                    .map_or(DEFAULT_DIMENSION, |c| c.dimension.as_str());
                let lbp = [lp[0].floor() as i32, lp[1].floor() as i32, lp[2].floor() as i32];
                let listener_sky = host
                    .level(dim)
                    .and_then(|lvl| lvl.brightness(host, LightLayer::Sky, lbp))
                    .unwrap_or(0);
                if listener_sky <= 9 {
                    let prox = (1.0 - (dist as f32 / 5.0)).clamp(0.0, 1.0);
                    let amount = ((0.14 + 0.18 * intensity) * prox).clamp(0.05, 0.34);
                    let lowpass = ((0.28 + 0.26 * intensity) * prox).clamp(0.10, 0.56);
                    let ticks = (10.0 + 10.0 * intensity * prox).round().clamp(8.0, 20.0) as u32;
                    *PENDING_CONCUSSION.lock().unwrap() = Some((amount, lowpass, ticks));
                }
            }
        }
        ACOUSTIC_CUE_SHOT_SEMI => {
            let _ = play_local_positional_channel(
                host,
                SoundChannel::Echo,
                SND_ECHO_CAVE,
                pos,
                0.30 * intensity,
                1.07 + 0.02 * j,
                (1 + prop_ticks).min(14),
            );
            let _ = play_local_positional_channel(
                host,
                SoundChannel::Echo,
                SND_ECHO_OPEN,
                pos,
                0.46 * intensity,
                0.98 + 0.02 * j,
                (3 + prop_ticks).min(16),
            );
        }
        ACOUSTIC_CUE_SHOT_OPEN => {
            let (vol, pitch, after) = if dist <= 32.0 {
                let base_delay = if intensity >= 0.72 { 2 } else { 3 };
                (
                    0.40 + 0.50 * intensity,
                    0.98 + 0.025 * j,
                    base_delay + prop_ticks,
                )
            } else {
                // Distant listeners (>32..96m) hear the rolling valley report arrive at speed of sound.
                let fade = (1.0 - ((dist - 32.0) / 68.0) as f32).clamp(0.16, 1.0);
                (
                    (1.55 + 0.45 * intensity) * fade,
                    0.91 + 0.025 * j,
                    prop_ticks.clamp(2, 12),
                )
            };
            let _ = play_local_positional_channel(
                host,
                SoundChannel::Echo,
                SND_ECHO_OPEN,
                pos,
                vol,
                pitch,
                after,
            );
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitch_jitter_stays_bounded_in_minus_one_to_one() {
        for salt in [0_u64, 1, 17, 999_983, u64::MAX] {
            let j = client_pitch_jitter(salt);
            assert!((-1.0..=1.0).contains(&j), "jitter out of bounds: {j}");
        }
    }
}
