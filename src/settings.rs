//! Client configuration page (`Options > Mods > Just a Rifle > Config`) & per-channel audio mixer.
//!
//! Registers 33 typed settings (`Toggle`, `Choice`, `Range`, `Color`) via
//! `Registrar::register_setting`, caches a fast `Copy` snapshot updated on `Hooks::setting_changed`
//! and world entry, and mixes both client-local foley and incoming world level sounds per player.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use mod_api::{ClientHost, Registrar, SettingError, SettingSpec, SoundSource};

use crate::constants::*;

// ---------------------------------------------------------------------------
// Setting Keys (`a-z`, `0-9`, `_`, `-`, `.`, <= 64 bytes)
// ---------------------------------------------------------------------------

pub(crate) const KEY_AUDIO_MASTER: &str = "audio.master";
pub(crate) const KEY_AUDIO_FIRE: &str = "audio.fire";
pub(crate) const KEY_AUDIO_BASS_THUMP: &str = "audio.bass_thump";
pub(crate) const KEY_AUDIO_FIRE_TONE: &str = "audio.fire_tone";
pub(crate) const KEY_AUDIO_ECHO: &str = "audio.echo";
pub(crate) const KEY_AUDIO_CONCUSSION: &str = "audio.concussion";
pub(crate) const KEY_AUDIO_MECHANISM: &str = "audio.mechanism";
pub(crate) const KEY_AUDIO_RELOAD: &str = "audio.reload";
pub(crate) const KEY_AUDIO_HANDLING: &str = "audio.handling";
pub(crate) const KEY_AUDIO_CASINGS: &str = "audio.casings";
pub(crate) const KEY_AUDIO_IMPACTS: &str = "audio.impacts";

pub(crate) const KEY_AUDIO_FEEDBACK: &str = "audio.feedback";
pub(crate) const KEY_FEEDBACK_HIT_SOUND: &str = "feedback.hit_sound";
pub(crate) const KEY_FEEDBACK_TARGET_CHIME: &str = "feedback.target_chime";
pub(crate) const KEY_FEEDBACK_DRY_CLICK: &str = "feedback.dry_click";

pub(crate) const KEY_CONTROLS_ADS_MODE: &str = "controls.ads_mode";
pub(crate) const KEY_CONTROLS_AUTO_RELOAD: &str = "controls.auto_reload";
pub(crate) const KEY_CONTROLS_ADS_SENS: &str = "controls.ads_sensitivity";

pub(crate) const KEY_CAMERA_ADS_ZOOM: &str = "camera.ads_zoom";
pub(crate) const KEY_CAMERA_RECOIL_KICK: &str = "camera.recoil_kick";
pub(crate) const KEY_CAMERA_WEAPON_SWAY: &str = "camera.weapon_sway";

pub(crate) const KEY_HUD_AMMO_POS: &str = "hud.ammo_position";
pub(crate) const KEY_HUD_SHOW_RESERVE: &str = "hud.show_reserve";
pub(crate) const KEY_HUD_ACCENT_COLOR: &str = "hud.accent_color";
pub(crate) const KEY_HUD_ADS_VIGNETTE: &str = "hud.ads_vignette";
pub(crate) const KEY_HUD_HIDE_CROSSHAIR: &str = "hud.hide_crosshair_ads";
pub(crate) const KEY_HUD_HITMARKER: &str = "hud.hitmarker";
pub(crate) const KEY_HUD_RICH_TOOLTIPS: &str = "hud.rich_tooltips";

pub(crate) const KEY_WORLD_MAX_CASINGS: &str = "world.max_casings";
pub(crate) const KEY_WORLD_CASING_LIFETIME: &str = "world.casing_lifetime";
pub(crate) const KEY_WORLD_MAX_HOLES: &str = "world.max_bullet_holes";
pub(crate) const KEY_WORLD_HOLE_LIFETIME: &str = "world.bullet_hole_lifetime";
pub(crate) const KEY_WORLD_BACK_SLING: &str = "world.back_sling";

// ---------------------------------------------------------------------------
// Strongly-Typed Option Enums & Snapshot
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FireTone {
    Balanced,
    DeepFrontier,
    CrispCrack,
}

impl FireTone {
    pub(crate) const fn from_index(idx: u32) -> Self {
        match idx {
            1 => Self::DeepFrontier,
            2 => Self::CrispCrack,
            _ => Self::Balanced,
        }
    }

    pub(crate) const fn pitch_mul(self) -> f32 {
        match self {
            Self::Balanced => 1.00,
            Self::DeepFrontier => 0.91,
            Self::CrispCrack => 1.08,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HitConfirmSound {
    Subtle,
    Chime,
    Click,
    Off,
}

impl HitConfirmSound {
    pub(crate) const fn from_index(idx: u32) -> Self {
        match idx {
            1 => Self::Chime,
            2 => Self::Click,
            3 => Self::Off,
            _ => Self::Subtle,
        }
    }

    pub(crate) const fn sound_spec(self) -> Option<(&'static str, f32, f32)> {
        match self {
            Self::Subtle => Some((SND_HIT_CONFIRM_SUBTLE, 0.44, 1.26)),
            Self::Chime => Some((SND_HIT_CONFIRM_CHIME, 0.56, 1.54)),
            Self::Click => Some((SND_HIT_CLICK, 0.48, 1.00)),
            Self::Off => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TargetChimeMode {
    InnerRings,
    BullseyeOnly,
    Off,
}

impl TargetChimeMode {
    pub(crate) const fn from_index(idx: u32) -> Self {
        match idx {
            1 => Self::BullseyeOnly,
            2 => Self::Off,
            _ => Self::InnerRings,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AdsMode {
    Hold,
    Toggle,
}

impl AdsMode {
    pub(crate) const fn from_index(idx: u32) -> Self {
        match idx {
            1 => Self::Toggle,
            _ => Self::Hold,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HudPosition {
    HotbarRight,
    HotbarLeft,
    BottomRight,
    BottomLeft,
    Hidden,
}

impl HudPosition {
    pub(crate) const fn from_index(idx: u32) -> Self {
        match idx {
            1 => Self::HotbarLeft,
            2 => Self::BottomRight,
            3 => Self::BottomLeft,
            4 => Self::Hidden,
            _ => Self::HotbarRight,
        }
    }

    /// Computes `[x0, y0]` top-left coordinates for a HUD counter box of `[box_w, box_h]` on a
    /// `[screen_w, screen_h]` GUI canvas, or `None` when `Hidden`.
    pub(crate) fn layout(
        self,
        screen_w: f32,
        screen_h: f32,
        box_w: f32,
        box_h: f32,
    ) -> Option<[f32; 2]> {
        match self {
            Self::HotbarRight => Some([
                (screen_w * 0.5 + 98.0).min(screen_w - box_w - 6.0),
                screen_h - 19.0,
            ]),
            Self::HotbarLeft => Some([(screen_w * 0.5 - 98.0 - box_w).max(6.0), screen_h - 19.0]),
            Self::BottomRight => Some([
                (screen_w - box_w - 8.0).max(6.0),
                (screen_h - box_h - 6.0).max(6.0),
            ]),
            Self::BottomLeft => Some([8.0, (screen_h - box_h - 6.0).max(6.0)]),
            Self::Hidden => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BackSlingStyle {
    Auto,
    FlipShoulder,
    Hidden,
}

impl BackSlingStyle {
    pub(crate) const fn from_index(idx: u32) -> Self {
        match idx {
            1 => Self::FlipShoulder,
            2 => Self::Hidden,
            _ => Self::Auto,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SoundChannel {
    FireBlast,
    BassThump,
    Echo,
    Mechanism,
    Reload,
    Handling,
    Casings,
    WorldImpact,
    Feedback,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ClientSettings {
    pub(crate) vol_master: f32,
    pub(crate) vol_fire: f32,
    pub(crate) vol_bass_thump: f32,
    pub(crate) fire_tone: FireTone,
    pub(crate) vol_echo: f32,
    pub(crate) blast_concussion: bool,
    pub(crate) vol_mechanism: f32,
    pub(crate) vol_reload: f32,
    pub(crate) vol_handling: f32,
    pub(crate) vol_casings: f32,
    pub(crate) vol_impacts: f32,

    pub(crate) vol_feedback: f32,
    pub(crate) hit_confirm_sound: HitConfirmSound,
    pub(crate) target_chime_mode: TargetChimeMode,
    pub(crate) dry_click_alert: bool,

    pub(crate) ads_mode: AdsMode,
    pub(crate) auto_reload: bool,
    pub(crate) ads_sensitivity_mul: f32,

    pub(crate) ads_zoom_mul: f32,
    pub(crate) camera_recoil_mul: f32,
    pub(crate) weapon_sway_mul: f32,

    pub(crate) hud_position: HudPosition,
    pub(crate) hud_show_reserve: bool,
    pub(crate) hud_accent_color: u32,
    pub(crate) ads_vignette_mul: f32,
    pub(crate) hide_crosshair_ads: bool,
    pub(crate) hud_hitmarker: bool,
    pub(crate) hud_rich_tooltips: bool,

    pub(crate) max_casings: usize,
    pub(crate) casing_lifetime_secs: f32,
    pub(crate) max_bullet_holes: usize,
    pub(crate) bullet_hole_lifetime_secs: f32,
    pub(crate) back_sling_style: BackSlingStyle,
}

impl Default for ClientSettings {
    fn default() -> Self {
        Self {
            vol_master: 1.0,
            vol_fire: 1.0,
            vol_bass_thump: 1.0,
            fire_tone: FireTone::Balanced,
            vol_echo: 1.0,
            blast_concussion: true,
            vol_mechanism: 1.0,
            vol_reload: 1.0,
            vol_handling: 1.0,
            vol_casings: 1.0,
            vol_impacts: 1.0,

            vol_feedback: 1.0,
            hit_confirm_sound: HitConfirmSound::Subtle,
            target_chime_mode: TargetChimeMode::InnerRings,
            dry_click_alert: true,

            ads_mode: AdsMode::Hold,
            auto_reload: false,
            ads_sensitivity_mul: 1.0,

            ads_zoom_mul: 1.0,
            camera_recoil_mul: 1.0,
            weapon_sway_mul: 1.0,

            hud_position: HudPosition::HotbarRight,
            hud_show_reserve: true,
            hud_accent_color: AMMO_BAR_RGB,
            ads_vignette_mul: 1.0,
            hide_crosshair_ads: true,
            hud_hitmarker: false,
            hud_rich_tooltips: true,

            max_casings: MAX_SPENT_CASINGS,
            casing_lifetime_secs: CASING_LIFETIME_SECS,
            max_bullet_holes: MAX_BULLET_HOLES,
            bullet_hole_lifetime_secs: BULLET_HOLE_LIFETIME_SECS,
            back_sling_style: BackSlingStyle::Auto,
        }
    }
}

static CLIENT_SETTINGS: OnceLock<Mutex<ClientSettings>> = OnceLock::new();
static SETTINGS_DIRTY: AtomicBool = AtomicBool::new(true);

fn settings_lock() -> &'static Mutex<ClientSettings> {
    CLIENT_SETTINGS.get_or_init(|| Mutex::new(ClientSettings::default()))
}

pub(crate) fn client_settings() -> ClientSettings {
    *settings_lock().lock().unwrap()
}

pub(crate) fn mark_settings_dirty() {
    SETTINGS_DIRTY.store(true, Ordering::Relaxed);
}

pub(crate) fn refresh_settings_if_dirty(host: &dyn ClientHost) {
    if SETTINGS_DIRTY.swap(false, Ordering::Relaxed) {
        refresh_client_settings(host);
    }
}

fn pct(host: &dyn ClientHost, key: &str, default_pct: f64, min_pct: f64, max_pct: f64) -> f32 {
    let raw = host
        .setting_f64(key)
        .unwrap_or(default_pct)
        .clamp(min_pct, max_pct);
    (raw * 0.01) as f32
}

pub(crate) fn refresh_client_settings(host: &dyn ClientHost) {
    let next = ClientSettings {
        vol_master: pct(host, KEY_AUDIO_MASTER, 100.0, 0.0, 200.0),
        vol_fire: pct(host, KEY_AUDIO_FIRE, 100.0, 0.0, 200.0),
        vol_bass_thump: pct(host, KEY_AUDIO_BASS_THUMP, 100.0, 0.0, 200.0),
        fire_tone: FireTone::from_index(host.setting_choice(KEY_AUDIO_FIRE_TONE).unwrap_or(0)),
        vol_echo: pct(host, KEY_AUDIO_ECHO, 100.0, 0.0, 200.0),
        blast_concussion: host.setting_bool(KEY_AUDIO_CONCUSSION).unwrap_or(true),
        vol_mechanism: pct(host, KEY_AUDIO_MECHANISM, 100.0, 0.0, 200.0),
        vol_reload: pct(host, KEY_AUDIO_RELOAD, 100.0, 0.0, 200.0),
        vol_handling: pct(host, KEY_AUDIO_HANDLING, 100.0, 0.0, 200.0),
        vol_casings: pct(host, KEY_AUDIO_CASINGS, 100.0, 0.0, 200.0),
        vol_impacts: pct(host, KEY_AUDIO_IMPACTS, 100.0, 0.0, 200.0),

        vol_feedback: pct(host, KEY_AUDIO_FEEDBACK, 100.0, 0.0, 200.0),
        hit_confirm_sound: HitConfirmSound::from_index(
            host.setting_choice(KEY_FEEDBACK_HIT_SOUND).unwrap_or(0),
        ),
        target_chime_mode: TargetChimeMode::from_index(
            host.setting_choice(KEY_FEEDBACK_TARGET_CHIME).unwrap_or(0),
        ),
        dry_click_alert: host.setting_bool(KEY_FEEDBACK_DRY_CLICK).unwrap_or(true),

        ads_mode: AdsMode::from_index(host.setting_choice(KEY_CONTROLS_ADS_MODE).unwrap_or(0)),
        auto_reload: host.setting_bool(KEY_CONTROLS_AUTO_RELOAD).unwrap_or(false),
        ads_sensitivity_mul: pct(host, KEY_CONTROLS_ADS_SENS, 100.0, 20.0, 150.0),

        ads_zoom_mul: pct(host, KEY_CAMERA_ADS_ZOOM, 100.0, 0.0, 150.0),
        camera_recoil_mul: pct(host, KEY_CAMERA_RECOIL_KICK, 100.0, 0.0, 200.0),
        weapon_sway_mul: pct(host, KEY_CAMERA_WEAPON_SWAY, 100.0, 0.0, 200.0),

        hud_position: HudPosition::from_index(host.setting_choice(KEY_HUD_AMMO_POS).unwrap_or(0)),
        hud_show_reserve: host.setting_bool(KEY_HUD_SHOW_RESERVE).unwrap_or(true),
        hud_accent_color: host
            .setting_color(KEY_HUD_ACCENT_COLOR)
            .unwrap_or(AMMO_BAR_RGB)
            & 0xFF_FFFF,
        ads_vignette_mul: pct(host, KEY_HUD_ADS_VIGNETTE, 100.0, 0.0, 150.0),
        hide_crosshair_ads: host.setting_bool(KEY_HUD_HIDE_CROSSHAIR).unwrap_or(true),
        hud_hitmarker: host.setting_bool(KEY_HUD_HITMARKER).unwrap_or(false),
        hud_rich_tooltips: host.setting_bool(KEY_HUD_RICH_TOOLTIPS).unwrap_or(true),

        max_casings: host
            .setting_f64(KEY_WORLD_MAX_CASINGS)
            .unwrap_or(MAX_SPENT_CASINGS as f64)
            .round()
            .clamp(0.0, 96.0) as usize,
        casing_lifetime_secs: host
            .setting_f64(KEY_WORLD_CASING_LIFETIME)
            .unwrap_or(f64::from(CASING_LIFETIME_SECS))
            .clamp(2.0, 60.0) as f32,
        max_bullet_holes: host
            .setting_f64(KEY_WORLD_MAX_HOLES)
            .unwrap_or(MAX_BULLET_HOLES as f64)
            .round()
            .clamp(0.0, 192.0) as usize,
        bullet_hole_lifetime_secs: host
            .setting_f64(KEY_WORLD_HOLE_LIFETIME)
            .unwrap_or(f64::from(BULLET_HOLE_LIFETIME_SECS))
            .clamp(5.0, 120.0) as f32,
        back_sling_style: BackSlingStyle::from_index(
            host.setting_choice(KEY_WORLD_BACK_SLING).unwrap_or(0),
        ),
    };
    *settings_lock().lock().unwrap() = next;
}

// ---------------------------------------------------------------------------
// Per-Channel Client Audio Mixer
// ---------------------------------------------------------------------------

/// Maps a local sound ID (`SND_*`) to its default `SoundChannel`.
pub(crate) fn default_channel_for_local_sound(event_id: &str) -> SoundChannel {
    match event_id {
        SND_FIRE => SoundChannel::FireBlast,
        SND_THUMP => SoundChannel::BassThump,
        SND_ECHO_OPEN | SND_ECHO_CAVE => SoundChannel::Echo,
        SND_BULLET_CRACK | SND_BULLET_WHIZ | SND_IMPACT_FLESH | SND_IMPACT_ARMOR => {
            SoundChannel::WorldImpact
        }
        SND_RELOAD_INTRO | SND_RELOAD_INSERT | SND_RELOAD_END => SoundChannel::Reload,
        SND_DRAW | SND_SLING | SND_INSPECT => SoundChannel::Handling,
        SND_SHELL_DROP => SoundChannel::Casings,
        SND_LEVER | SND_COCK | SND_MISFIRE_WET => SoundChannel::Mechanism,
        SND_DRY_FIRE | SND_HIT_CLICK => SoundChannel::Feedback,
        _ => SoundChannel::Feedback,
    }
}

/// Applies the player's master + channel volume sliders and gunshot tone preference.
/// Caps positional gain per channel so `200%` sliders never inflate OpenAL's `16 * volume`
/// attenuation radius abnormally or let `BassThump` out-range `FireBlast`.
/// Returns `None` when the effective gain is muted (`<= 0.001`) so callers skip playback cleanly.
pub(crate) fn mix_client_sound(
    cfg: &ClientSettings,
    channel: SoundChannel,
    volume: f32,
    pitch: f32,
) -> Option<(f32, f32)> {
    let ch_gain = match channel {
        SoundChannel::FireBlast => cfg.vol_fire,
        SoundChannel::BassThump => cfg.vol_fire * cfg.vol_bass_thump,
        SoundChannel::Echo => cfg.vol_fire * cfg.vol_echo,
        SoundChannel::Mechanism => cfg.vol_mechanism,
        SoundChannel::Reload => cfg.vol_reload,
        SoundChannel::Handling => cfg.vol_handling,
        SoundChannel::Casings => cfg.vol_casings,
        SoundChannel::WorldImpact => cfg.vol_impacts,
        SoundChannel::Feedback => cfg.vol_feedback,
    };
    let raw_vol = volume * cfg.vol_master * ch_gain;
    if !raw_vol.is_finite() || raw_vol <= 0.001 {
        return None;
    }
    let out_vol = match channel {
        SoundChannel::FireBlast => raw_vol.min(5.0),
        SoundChannel::Echo => raw_vol.min(4.5),
        SoundChannel::BassThump => {
            let fire_ref_cap = (volume * 2.0 * cfg.vol_master * cfg.vol_fire).max(0.0);
            raw_vol.min(fire_ref_cap).min(2.5)
        }
        _ => raw_vol.min(2.0),
    };
    if out_vol <= 0.001 {
        return None;
    }
    let tone_mul = match channel {
        SoundChannel::FireBlast | SoundChannel::BassThump | SoundChannel::Echo => {
            cfg.fire_tone.pitch_mul()
        }
        _ => 1.0,
    };
        _ => 1.0,
    };
    let out_pitch = (pitch * tone_mul).clamp(0.25, 2.50);
    Some((out_vol, out_pitch))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LevelSoundDecision {
    Ignore,
    Cancel,
    Mix(SoundChannel),
}

/// Classifies incoming positional level sounds on the client (`PlayLevelSoundEvent.AtPosition`)
/// so each player's own sliders control gunfire, lever cycling, material bullet impacts, water
/// entry splashes, and `minecraft:target` bullseye chimes without affecting unrelated Vanilla sounds.
pub(crate) fn classify_incoming_level_sound(
    cfg: &ClientSettings,
    namespace: &str,
    path: &str,
    source: SoundSource,
    original_pitch: f32,
) -> LevelSoundDecision {
    if namespace == MOD_ID {
        let ch = match path {
            "item.winchester_fire" => SoundChannel::FireBlast,
            "item.winchester_thump" => SoundChannel::BassThump,
            "item.winchester_echo_open" | "item.winchester_echo_cave" => SoundChannel::Echo,
            "item.winchester_bullet_crack"
            | "item.winchester_bullet_whiz"
            | "item.winchester_impact_flesh"
            | "item.winchester_impact_armor" => SoundChannel::WorldImpact,
            "animation.winchester_reload_intro"
            | "animation.winchester_reload_insert"
            | "animation.winchester_reload_end" => SoundChannel::Reload,
            "animation.winchester_draw"
            | "animation.winchester_sling"
            | "animation.winchester_inspect" => SoundChannel::Handling,
            "item.winchester_shell_drop" => SoundChannel::Casings,
            "item.winchester_hit_click" => SoundChannel::Feedback,
            _ => SoundChannel::Mechanism,
        };
        return LevelSoundDecision::Mix(ch);
    }

    // Bullet terrain impacts, ricochets, crystal chimes, candle snuffs, water entry splashes,
    // and `minecraft:target` bullseye chimes are emitted on `SoundSource::Players` (whereas
    // Vanilla mining/footsteps/candles use `Blocks`, bobbers use `Neutral`, and Note Blocks use `Records`).
    if namespace == "minecraft" && source == SoundSource::Players {
        if path == "entity.fishing_bobber.splash" {
            return LevelSoundDecision::Mix(SoundChannel::WorldImpact);
        }
        if path.starts_with("block.")
            && !path.starts_with("block.anvil.")
            && !path.starts_with("block.grindstone.")
            && !path.starts_with("block.smithing_table.")
            && !path.starts_with("block.enchantment_table.")
        {
            if path == "block.note_block.chime" {
                return match cfg.target_chime_mode {
                    TargetChimeMode::Off => LevelSoundDecision::Cancel,
                    TargetChimeMode::BullseyeOnly if original_pitch < 1.40 => {
                        LevelSoundDecision::Cancel
                    }
                    TargetChimeMode::BullseyeOnly | TargetChimeMode::InnerRings => {
                        LevelSoundDecision::Mix(SoundChannel::Feedback)
                    }
                };
            }
            return LevelSoundDecision::Mix(SoundChannel::WorldImpact);
        }
    }

    LevelSoundDecision::Ignore
}

// ---------------------------------------------------------------------------
// Setting Registration Specs (`Options > Mods > Just a Rifle > Config`)
// ---------------------------------------------------------------------------

fn label_key(key: &str) -> String {
    format!("config.{MOD_ID}.{key}")
}

fn tip_key(key: &str) -> String {
    format!("config.{MOD_ID}.{key}.tooltip")
}

fn range_pct(key: &str, min: f64, max: f64, step: f64, default: f64) -> SettingSpec {
    SettingSpec::range(key, label_key(key), min, max, step, default).tooltip(tip_key(key))
}

fn toggle_opt(key: &str, default: bool) -> SettingSpec {
    SettingSpec::toggle(key, label_key(key), default).tooltip(tip_key(key))
}

fn choice_opt(key: &str, suffixes: &[&str], default: usize) -> SettingSpec {
    let options: Vec<String> = suffixes
        .iter()
        .map(|s| format!("config.{MOD_ID}.{key}.{s}"))
        .collect();
    SettingSpec::choice(key, label_key(key), options, default).tooltip(tip_key(key))
}

pub(crate) fn setting_specs() -> Vec<SettingSpec> {
    vec![
        // 1. Audio — Per-Channel Volume & Gunshot Tone
        range_pct(KEY_AUDIO_MASTER, 0.0, 200.0, 5.0, 100.0),
        range_pct(KEY_AUDIO_FIRE, 0.0, 200.0, 5.0, 100.0),
        range_pct(KEY_AUDIO_BASS_THUMP, 0.0, 200.0, 5.0, 100.0),
        choice_opt(KEY_AUDIO_FIRE_TONE, &["balanced", "deep", "crisp"], 0),
        range_pct(KEY_AUDIO_ECHO, 0.0, 200.0, 5.0, 100.0),
        toggle_opt(KEY_AUDIO_CONCUSSION, true),
        range_pct(KEY_AUDIO_MECHANISM, 0.0, 200.0, 5.0, 100.0),
        range_pct(KEY_AUDIO_RELOAD, 0.0, 200.0, 5.0, 100.0),
        range_pct(KEY_AUDIO_HANDLING, 0.0, 200.0, 5.0, 100.0),
        range_pct(KEY_AUDIO_CASINGS, 0.0, 200.0, 5.0, 100.0),
        range_pct(KEY_AUDIO_IMPACTS, 0.0, 200.0, 5.0, 100.0),
        // 2. Audio & Tactical Feedback ("Retorno Sonoro")
        range_pct(KEY_AUDIO_FEEDBACK, 0.0, 200.0, 5.0, 100.0),
        choice_opt(
            KEY_FEEDBACK_HIT_SOUND,
            &["subtle", "chime", "click", "off"],
            0,
        ),
        choice_opt(
            KEY_FEEDBACK_TARGET_CHIME,
            &["inner", "bullseye", "off"],
            0,
        ),
        toggle_opt(KEY_FEEDBACK_DRY_CLICK, true),
        // 3. Controls & Ergonomics
        choice_opt(KEY_CONTROLS_ADS_MODE, &["hold", "toggle"], 0),
        toggle_opt(KEY_CONTROLS_AUTO_RELOAD, false),
        range_pct(KEY_CONTROLS_ADS_SENS, 20.0, 150.0, 5.0, 100.0),
        // 4. Camera, Sights & 1st-Person Motion
        range_pct(KEY_CAMERA_ADS_ZOOM, 0.0, 150.0, 5.0, 100.0),
        range_pct(KEY_CAMERA_RECOIL_KICK, 0.0, 200.0, 10.0, 100.0),
        range_pct(KEY_CAMERA_WEAPON_SWAY, 0.0, 200.0, 10.0, 100.0),
        // 5. HUD, Tooltips & Visual Feedback
        choice_opt(
            KEY_HUD_AMMO_POS,
            &[
                "hotbar_right",
                "hotbar_left",
                "bottom_right",
                "bottom_left",
                "hidden",
            ],
            0,
        ),
        toggle_opt(KEY_HUD_SHOW_RESERVE, true),
        SettingSpec::color(
            KEY_HUD_ACCENT_COLOR,
            label_key(KEY_HUD_ACCENT_COLOR),
            AMMO_BAR_RGB,
        )
        .tooltip(tip_key(KEY_HUD_ACCENT_COLOR)),
        range_pct(KEY_HUD_ADS_VIGNETTE, 0.0, 150.0, 10.0, 100.0),
        toggle_opt(KEY_HUD_HIDE_CROSSHAIR, true),
        toggle_opt(KEY_HUD_HITMARKER, false),
        toggle_opt(KEY_HUD_RICH_TOOLTIPS, true),
        // 6. World Effects & Performance
        range_pct(
            KEY_WORLD_MAX_CASINGS,
            0.0,
            96.0,
            8.0,
            MAX_SPENT_CASINGS as f64,
        ),
        range_pct(
            KEY_WORLD_CASING_LIFETIME,
            2.0,
            60.0,
            2.0,
            f64::from(CASING_LIFETIME_SECS),
        ),
        range_pct(
            KEY_WORLD_MAX_HOLES,
            0.0,
            192.0,
            16.0,
            MAX_BULLET_HOLES as f64,
        ),
        range_pct(
            KEY_WORLD_HOLE_LIFETIME,
            5.0,
            120.0,
            5.0,
            f64::from(BULLET_HOLE_LIFETIME_SECS),
        ),
        choice_opt(KEY_WORLD_BACK_SLING, &["auto", "flip", "hidden"], 0),
    ]
}

pub(crate) fn register_all_settings(r: &mut dyn Registrar) -> Result<(), SettingError> {
    for spec in setting_specs() {
        r.register_setting(spec)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn all_registered_setting_specs_validate_with_unique_keys() {
        let specs = setting_specs();
        assert_eq!(specs.len(), 33);
        let mut seen = BTreeSet::new();
        for spec in &specs {
            assert_eq!(spec.check(), Ok(()), "invalid spec: {}", spec.key);
            assert!(
                seen.insert(spec.key.clone()),
                "duplicate setting key: {}",
                spec.key
            );
            assert!(spec.tooltip.is_some(), "missing tooltip on {}", spec.key);
        }
    }

    #[test]
    fn audio_mixer_scales_mutes_and_shifts_tone_independently() {
        let mut cfg = ClientSettings::default();
        assert_eq!(
            mix_client_sound(&cfg, SoundChannel::FireBlast, 0.80, 1.00),
            Some((0.80, 1.00))
        );

        // Deep frontier tone lowers gunshot pitch while leaving reload pitch untouched.
        cfg.fire_tone = FireTone::DeepFrontier;
        cfg.vol_bass_thump = 1.50;
        let (thump_v, thump_p) =
            mix_client_sound(&cfg, SoundChannel::BassThump, 1.00, 1.00).unwrap();
        assert!((thump_v - 1.50).abs() < 1e-5);
        assert!((thump_p - 0.91).abs() < 1e-5);
        let (_, reload_p) = mix_client_sound(&cfg, SoundChannel::Reload, 0.90, 1.00).unwrap();
        assert!((reload_p - 1.00).abs() < 1e-5);

        // Muting a single channel or master cleanly suppresses playback (`None`).
        cfg.vol_reload = 0.0;
        assert_eq!(
            mix_client_sound(&cfg, SoundChannel::Reload, 0.90, 1.00),
            None
        );
        assert!(mix_client_sound(&cfg, SoundChannel::Feedback, 0.70, 1.20).is_some());

        cfg.vol_master = 0.0;
        assert_eq!(
            mix_client_sound(&cfg, SoundChannel::Feedback, 0.70, 1.20),
            None
        );

        // Extreme 200% sliders stay bounded so BassThump never out-ranges FireBlast.
        let mut loud = ClientSettings::default();
        loud.vol_master = 2.0;
        loud.vol_fire = 2.0;
        loud.vol_bass_thump = 2.0;
        let (fire_v, _) = mix_client_sound(&loud, SoundChannel::FireBlast, 2.50, 1.0).unwrap();
        let (thump_v, _) = mix_client_sound(&loud, SoundChannel::BassThump, 1.15, 1.0).unwrap();
        assert!(thump_v <= 2.50 && thump_v < fire_v);
        assert_eq!(HitConfirmSound::Click.sound_spec(), Some((SND_HIT_CLICK, 0.48, 1.00)));
        assert_eq!(default_channel_for_local_sound(SND_SLING), SoundChannel::Handling);
        assert_eq!(default_channel_for_local_sound(SND_DRY_FIRE), SoundChannel::Feedback);
        assert_eq!(default_channel_for_local_sound(SND_ECHO_OPEN), SoundChannel::Echo);
        assert_eq!(default_channel_for_local_sound(SND_ECHO_CAVE), SoundChannel::Echo);
        assert_eq!(
            default_channel_for_local_sound(SND_BULLET_CRACK),
            SoundChannel::WorldImpact
        );
        assert_eq!(
            default_channel_for_local_sound(SND_IMPACT_ARMOR),
            SoundChannel::WorldImpact
        );
    }

    #[test]
    fn incoming_level_sound_classifier_isolates_mod_and_bullet_sounds() {
        let mut cfg = ClientSettings::default();
        assert_eq!(
            classify_incoming_level_sound(
                &cfg,
                MOD_ID,
                "item.winchester_fire",
                SoundSource::Players,
                1.0
            ),
            LevelSoundDecision::Mix(SoundChannel::FireBlast)
        );

        // Vanilla block mining on `SoundSource::Blocks` and Note Blocks on `Records` stay untouched.
        assert_eq!(
            classify_incoming_level_sound(
                &cfg,
                "minecraft",
                "block.stone.hit",
                SoundSource::Blocks,
                1.0
            ),
            LevelSoundDecision::Ignore
        );
        assert_eq!(
            classify_incoming_level_sound(
                &cfg,
                "minecraft",
                "block.note_block.chime",
                SoundSource::Records,
                1.24
            ),
            LevelSoundDecision::Ignore
        );
        assert_eq!(
            classify_incoming_level_sound(
                &cfg,
                "minecraft",
                "entity.fishing_bobber.splash",
                SoundSource::Neutral,
                1.0
            ),
            LevelSoundDecision::Ignore
        );
        assert_eq!(
            classify_incoming_level_sound(
                &cfg,
                "minecraft",
                "entity.fishing_bobber.splash",
                SoundSource::Players,
                1.42
            ),
            LevelSoundDecision::Mix(SoundChannel::WorldImpact)
        );
        assert_eq!(
            classify_incoming_level_sound(
                &cfg,
                "minecraft",
                "block.anvil.use",
                SoundSource::Players,
                1.0
            ),
            LevelSoundDecision::Ignore
        );

        // Bullet impact & Target bullseye on `SoundSource::Players` obey `impacts` and `target_chime_mode`.
        assert_eq!(
            classify_incoming_level_sound(
                &cfg,
                "minecraft",
                "block.lantern.break",
                SoundSource::Players,
                1.78
            ),
            LevelSoundDecision::Mix(SoundChannel::WorldImpact)
        );
        assert_eq!(
            classify_incoming_level_sound(
                &cfg,
                "minecraft",
                "block.note_block.chime",
                SoundSource::Players,
                1.24
            ),
            LevelSoundDecision::Mix(SoundChannel::Feedback)
        );

        cfg.target_chime_mode = TargetChimeMode::BullseyeOnly;
        assert_eq!(
            classify_incoming_level_sound(
                &cfg,
                "minecraft",
                "block.note_block.chime",
                SoundSource::Players,
                1.24
            ),
            LevelSoundDecision::Cancel
        );
        assert_eq!(
            classify_incoming_level_sound(
                &cfg,
                "minecraft",
                "block.note_block.chime",
                SoundSource::Players,
                1.56
            ),
            LevelSoundDecision::Mix(SoundChannel::Feedback)
        );
    }

    #[test]
    fn hud_position_layouts_stay_on_screen() {
        for width in [52.0_f32, 88.0_f32] {
            for idx in 0..4 {
                let pos = HudPosition::from_index(idx);
                let [x, y] = pos.layout(427.0, 240.0, width, 15.0).unwrap();
                assert!(x >= 0.0 && x + width <= 427.0);
                assert!(y >= 0.0 && y + 15.0 <= 240.0);
            }
            assert_eq!(HudPosition::Hidden.layout(427.0, 240.0, width, 15.0), None);
        }
    }
}
