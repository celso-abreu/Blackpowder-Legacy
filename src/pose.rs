//! Second-order spring dynamics, TAC-style action state machine, 1st-person pose
//! evaluator, articulated rifle assembly renderer, and view-space kinematics.

use std::f32::consts::PI;
use std::sync::atomic::Ordering;

use mod_api::{BlockEntityDrawContext, ClientHost, EquipmentSlot, Hand, ModelRef, VarInt};

use crate::arms::LimbModel;
use crate::constants::*;
use crate::server::rounds;
use crate::skin::ResolvedSkin;
use crate::{
    aim_progress, holding_rifle_recently, RifleInput, CLIENT_SNEAKING, LAST_MUZZLE_VIEW_X,
    LAST_MUZZLE_VIEW_Y, LAST_MUZZLE_VIEW_Z, LAST_WORLD_FOV_BITS, RIFLE_KEY,
};

#[derive(Clone, Copy)]
pub(crate) struct SlungRifleState {
    pub(crate) last_tick: u64,
    pub(crate) hammer_uncocked: bool,
    pub(crate) main_arm_left: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct RifleModels {
    pub(crate) body: ModelRef,
    pub(crate) body_lod: ModelRef,
    pub(crate) lever: ModelRef,
    pub(crate) hammer: ModelRef,
    pub(crate) trigger: ModelRef,
    pub(crate) gate: ModelRef,
    pub(crate) cartridge: ModelRef,
    pub(crate) casing: ModelRef,
    pub(crate) bullet_hole: ModelRef,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ActionAnim {
    None,
    Draw {
        elapsed: f32,
    },
    FireCycle {
        elapsed: f32,
        keep_uncocked: bool,
        lever_sound_played: bool,
        cock_sound_played: bool,
        casing_ejected: bool,
    },
    DryClick {
        elapsed: f32,
    },
    ReloadIntro {
        elapsed: f32,
    },
    ReloadInsert {
        elapsed: f32,
    },
    ReloadEndNorm {
        elapsed: f32,
    },
    ReloadEndEmpty {
        elapsed: f32,
        lever_sound_played: bool,
        cock_sound_played: bool,
    },
    Inspect {
        elapsed: f32,
    },
}

/// Semi-implicit second-order spring-damper (`f` Hz natural frequency, `zeta` damping,
/// `r` initial velocity response). Sub-stepped so arbitrary frame deltas remain stable.
#[derive(Clone, Copy)]
pub(crate) struct SecondOrder {
    k1: f32,
    k2: f32,
    k3: f32,
    xp: f32,
    pub(crate) y: f32,
    yd: f32,
}

impl SecondOrder {
    pub(crate) fn new(f: f32, zeta: f32, r: f32, x0: f32) -> Self {
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

    pub(crate) fn step(&mut self, dt: f32, x: f32) -> f32 {
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

    pub(crate) fn reset(&mut self, x0: f32) {
        self.xp = x0;
        self.y = x0;
        self.yd = 0.0;
    }
}

pub(crate) struct ClientAnimState {
    pub(crate) aim_dyn: SecondOrder,
    pub(crate) recoil_dyn: SecondOrder,
    pub(crate) sprint_dyn: SecondOrder,
    pub(crate) action: ActionAnim,
    pub(crate) was_holding: bool,
    pub(crate) was_slung: bool,
    pub(crate) known_rounds: u32,
    pub(crate) hammer_uncocked: bool,
    pub(crate) walk_phase: f32,
    pub(crate) walk_speed: f32,
    pub(crate) idle_time: f32,
    pub(crate) shot_ads_factor: f32,
    pub(crate) shot_crouch_factor: f32,
    pub(crate) shot_roll_sign: f32,
    pub(crate) recoil_target_timer: Option<f32>,
    pub(crate) cam_recoil_timer: Option<f32>,
    pub(crate) cam_pitch_kick: f32,
    pub(crate) cam_yaw_kick: f32,
    pub(crate) jolt_timer: f32,
    pub(crate) jolt_yaw_sign: f32,
    rng: u32,
    pub(crate) skin_requested: bool,
    pub(crate) skin: ResolvedSkin,
}

impl ClientAnimState {
    pub(crate) fn new() -> Self {
        Self {
            aim_dyn: SecondOrder::new(3.8, 0.88, 1.05, 0.0),
            recoil_dyn: SecondOrder::new(5.6, 0.60, 2.35, 0.0),
            sprint_dyn: SecondOrder::new(3.4, 0.90, 0.75, 0.0),
            action: ActionAnim::None,
            was_holding: false,
            was_slung: false,
            known_rounds: TUBE_CAPACITY,
            hammer_uncocked: false,
            walk_phase: 0.0,
            walk_speed: 0.0,
            idle_time: 0.0,
            shot_ads_factor: 1.0,
            shot_crouch_factor: 1.0,
            shot_roll_sign: 1.0,
            recoil_target_timer: None,
            cam_recoil_timer: Some(1.0),
            cam_pitch_kick: 0.0,
            cam_yaw_kick: 0.0,
            jolt_timer: 0.0,
            jolt_yaw_sign: 1.0,
            rng: 0x6D2B_79F5,
            skin_requested: false,
            skin: ResolvedSkin::default(),
        }
    }

    fn next_unit_f32(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        ((self.rng >> 8) as f32) * (1.0 / 16_777_216.0)
    }

    pub(crate) fn blocks_ads(&self) -> bool {
        matches!(
            self.action,
            ActionAnim::Draw { .. }
                | ActionAnim::ReloadIntro { .. }
                | ActionAnim::ReloadInsert { .. }
                | ActionAnim::ReloadEndNorm { .. }
                | ActionAnim::ReloadEndEmpty { .. }
                | ActionAnim::Inspect { .. }
        )
    }

    pub(crate) fn on_shot(&mut self, remaining_rounds: u32) {
        let aim = self.aim_dyn.y.clamp(0.0, 1.0);
        let sneaking = CLIENT_SNEAKING.load(Ordering::Relaxed);
        let ads_factor = 1.0 - 0.32 * aim;
        let crouch_factor = if sneaking { 0.875 } else { 1.0 };
        let variance = 0.88 + 0.24 * self.next_unit_f32();
        let yaw_rand = self.next_unit_f32() * 2.0 - 1.0;

        self.known_rounds = remaining_rounds;
        self.shot_ads_factor = ads_factor;
        self.shot_crouch_factor = crouch_factor;
        self.shot_roll_sign = if yaw_rand >= 0.0 { 1.0 } else { -1.0 };
        self.recoil_target_timer = Some(0.0);

        self.cam_recoil_timer = Some(0.0);
        self.cam_pitch_kick = 5.2 * ads_factor * crouch_factor * variance;
        self.cam_yaw_kick = 0.68 * ads_factor * yaw_rand;

        self.jolt_timer = 0.065;
        self.jolt_yaw_sign = self.shot_roll_sign;

        let keep_uncocked = remaining_rounds == 0;
        if keep_uncocked {
            self.hammer_uncocked = true;
        }
        self.action = ActionAnim::FireCycle {
            elapsed: 0.0,
            keep_uncocked,
            lever_sound_played: false,
            cock_sound_played: false,
            casing_ejected: false,
        };
    }

    pub(crate) fn on_dry_click(&mut self, rounds: u32) {
        self.known_rounds = rounds;
        if rounds == 0 {
            self.hammer_uncocked = true;
        }
        self.action = ActionAnim::DryClick { elapsed: 0.0 };
    }
}

pub(crate) struct WeaponPose {
    pub(crate) gun_trans: [f32; 3],
    pub(crate) gun_rot: [f32; 3],
    pub(crate) lever_deg: f32,
    pub(crate) hammer_deg: f32,
    pub(crate) trigger_deg: f32,
    pub(crate) gate_deg: f32,
    pub(crate) show_cartridge: bool,
    pub(crate) cartridge_tex: [f32; 3],
    pub(crate) cartridge_rot: [f32; 3],
    pub(crate) show_casing: bool,
    pub(crate) casing_tex: [f32; 3],
    pub(crate) casing_rot: [f32; 3],
    pub(crate) right_hand_tex: [f32; 3],
    pub(crate) right_hand_rot: [f32; 3],
    pub(crate) left_hand_tex: [f32; 3],
    pub(crate) left_hand_rot: [f32; 3],
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn evaluate_weapon_pose(
    aim: f32,
    equip: f32,
    kick: f32,
    sprint_blend: f32,
    ads_factor: f32,
    crouch_factor: f32,
    roll_sign: f32,
    walk_phase: f32,
    walk_speed: f32,
    idle_time: f32,
    sway_mul: f32,
    hammer_uncocked: bool,
    action: ActionAnim,
) -> WeaponPose {
    let aim_clamped = aim.clamp(0.0, 1.0);
    let hip = 1.0 - aim_clamped;

    // Shoulder-pocket parabolic arc dip while raising/lowering iron sights.
    let arc_dip = -0.030 * (4.0 * aim_clamped * hip);
    let mut gun_trans = [
        ADS_OFFSET[0] * aim,
        ADS_OFFSET[1] * aim + arc_dip,
        ADS_OFFSET[2] * aim,
    ];
    let mut gun_rot = [0.0, 0.0, 3.2 * hip];

    // Subtle idle breathing + walk Lissajous bobbing (damped 88% in ADS, scaled by player config).
    let sway_scale = (0.12 + 0.88 * hip) * sway_mul.clamp(0.0, 2.0);
    gun_trans[0] += walk_phase.cos() * 0.013 * walk_speed * sway_scale;
    gun_trans[1] += (idle_time * 1.75).sin() * 0.0025 * sway_scale
        - (walk_phase * 2.0).sin().abs() * 0.011 * walk_speed * sway_scale;
    gun_rot[0] += (idle_time * 1.75 + 0.6).cos() * 0.32 * sway_scale;
    gun_rot[2] += walk_phase.sin() * 1.35 * walk_speed * sway_scale;

    // Smooth sprint low-ready carry across chest.
    if sprint_blend > 1e-3 {
        gun_trans[0] -= 0.105 * sprint_blend;
        gun_trans[1] -= 0.072 * sprint_blend;
        gun_trans[2] += 0.028 * sprint_blend;
        gun_rot[0] -= 11.5 * sprint_blend;
        gun_rot[1] += 25.0 * sprint_blend;
        gun_rot[2] += 13.5 * sprint_blend;
    }

    // Rear-stock-pivoted 2nd-order visual recoil kick.
    if kick.abs() > 1e-4 {
        let pitch_deg = 7.2 * kick * ads_factor * crouch_factor;
        let pitch_rad = pitch_deg * (PI / 180.0);
        let pivot = model_texel_to_hand(RECOIL_PIVOT_TEXELS);
        let py = HAND_BASE[1] - equip * 0.6 + pivot[1];
        let pz = HAND_BASE[2] + pivot[2];
        let dy = py - (py * pitch_rad.cos() - pz * pitch_rad.sin());
        let dz = pz - (py * pitch_rad.sin() + pz * pitch_rad.cos());

        gun_trans[1] += dy + 0.008 * kick * ads_factor;
        gun_trans[2] += dz + 0.095 * kick * ads_factor;
        gun_rot[0] += pitch_deg;
        gun_rot[1] += 0.85 * kick * ads_factor * roll_sign;
        gun_rot[2] += 1.30 * kick * ads_factor * roll_sign;
    }

    let mut lever_deg = 0.0_f32;
    let mut hammer_deg = if hammer_uncocked { -36.0 } else { 0.0 };
    let mut trigger_deg = 0.0_f32;
    let mut gate_deg = 0.0_f32;
    let mut show_cartridge = false;
    let mut cartridge_tex = [11.2, 8.2, 11.2];
    let mut cartridge_rot = [0.0, 0.0, 0.0];
    let mut show_casing = false;
    let mut casing_tex = EJECTOR_START_TEXELS;
    let mut casing_rot = [0.0, 0.0, 0.0];

    let base_right_tex = [
        GRIP_WRIST_TEXELS[0] - 0.20 * aim_clamped,
        GRIP_WRIST_TEXELS[1] - 0.35 * aim_clamped,
        GRIP_WRIST_TEXELS[2] + 0.10 * aim_clamped,
    ];
    let base_right_rot = [
        124.0 + 12.0 * aim_clamped,
        18.0 - 6.0 * aim_clamped,
        -30.0 + 4.0 * aim_clamped,
    ];
    let base_left_tex = [
        FORESTOCK_TEXELS[0],
        FORESTOCK_TEXELS[1] - 0.25 * aim_clamped,
        FORESTOCK_TEXELS[2],
    ];
    let base_left_rot = [
        118.0 + 4.0 * aim_clamped,
        -16.0 + 3.0 * aim_clamped,
        14.0 - 2.0 * aim_clamped,
    ];

    let mut right_hand_tex = base_right_tex;
    let mut right_hand_rot = base_right_rot;
    let mut left_hand_tex = base_left_tex;
    let mut left_hand_rot = base_left_rot;

    match action {
        ActionAnim::None => {}
        ActionAnim::Draw { elapsed } => {
            let p = smoothstep((elapsed / DRAW_DURATION_SECS).clamp(0.0, 1.0));
            let inv = 1.0 - p;
            gun_trans[0] += 0.075 * inv;
            gun_trans[1] -= 0.27 * inv;
            gun_trans[2] += 0.055 * inv;
            gun_rot[0] -= 24.0 * inv;
            gun_rot[1] -= 13.0 * inv;
            gun_rot[2] += 17.0 * inv;
        }
        ActionAnim::FireCycle {
            elapsed,
            keep_uncocked,
            ..
        } => {
            // Trigger pull + hammer fall (`0° -> -36°` in `50 ms`).
            let strike = (elapsed / 0.05).clamp(0.0, 1.0);
            trigger_deg = -20.0 * (1.0 - ((elapsed - 0.06).max(0.0) / 0.14).clamp(0.0, 1.0));
            hammer_deg = -36.0 * strike;

            if keep_uncocked {
                if (CASING_LAST_RISE_START_SECS..CASING_LAST_HANDOFF_SECS).contains(&elapsed) {
                    let u = ((elapsed - CASING_LAST_RISE_START_SECS)
                        / (CASING_LAST_HANDOFF_SECS - CASING_LAST_RISE_START_SECS))
                        .clamp(0.0, 1.0);
                    show_casing = true;
                    casing_tex = lerp3(EJECTOR_START_TEXELS, EJECTOR_HANDOFF_TEXELS, u);
                    casing_rot = [18.0 * u, -8.0 * u, 24.0 * u];
                }
            } else if elapsed >= LEVER_DELAY_SECS {
                if (CASING_RISE_START_SECS..CASING_HANDOFF_SECS).contains(&elapsed) {
                    let u = ((elapsed - CASING_RISE_START_SECS)
                        / (CASING_HANDOFF_SECS - CASING_RISE_START_SECS))
                        .clamp(0.0, 1.0);
                    show_casing = true;
                    casing_tex = lerp3(EJECTOR_START_TEXELS, EJECTOR_HANDOFF_TEXELS, u);
                    casing_rot = [18.0 * u, -8.0 * u, 24.0 * u];
                }
                let lt = ((elapsed - LEVER_DELAY_SECS) / LEVER_DURATION_SECS).clamp(0.0, 1.0);
                let lever_curve = if lt < 0.45 {
                    smoothstep(lt / 0.45)
                } else {
                    1.0 - smoothstep((lt - 0.45) / 0.55)
                };
                lever_deg = 48.0 * lever_curve;
                hammer_deg = if lt < 0.48 {
                    lerp(-36.0, 7.5, smoothstep(lt / 0.48))
                } else {
                    lerp(7.5, 0.0, smoothstep((lt - 0.48) / 0.52))
                };

                let damp = 1.0 - 0.40 * aim_clamped;
                gun_trans[1] -= 0.028 * lever_curve * damp;
                gun_trans[2] += 0.015 * lever_curve * damp;
                gun_rot[0] -= 2.4 * lever_curve * damp;
                gun_rot[2] += 7.5 * lever_curve * damp;

                right_hand_tex = [
                    base_right_tex[0],
                    base_right_tex[1] - 2.15 * lever_curve,
                    base_right_tex[2] - 1.95 * lever_curve,
                ];
                right_hand_rot[0] = base_right_rot[0] + 16.0 * lever_curve;
            }
        }
        ActionAnim::DryClick { elapsed } => {
            let p = 1.0 - (elapsed / 0.22).clamp(0.0, 1.0);
            trigger_deg = -20.0 * p;
            gun_trans[2] += 0.009 * p;
            gun_rot[0] -= 1.1 * p;
        }
        ActionAnim::ReloadIntro { elapsed } => {
            let open_secs = (RELOAD_OPEN_TICKS as f32) * 0.05;
            let p = smoothstep((elapsed / open_secs).clamp(0.0, 1.0));
            apply_reload_cradle(&mut gun_trans, &mut gun_rot, p);
            left_hand_tex = lerp3(base_left_tex, [12.10, 7.20, 12.20], p);
            left_hand_rot = lerp3(base_left_rot, [122.0, -12.0, 24.0], p);
        }
        ActionAnim::ReloadInsert { elapsed } => {
            apply_reload_cradle(&mut gun_trans, &mut gun_rot, 1.0);
            let shell_secs = (RELOAD_INSERT_TICKS as f32) * 0.05;
            let u = (elapsed / shell_secs).clamp(0.0, 1.0);

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

            let seat_bump = (-((u - 0.62) * 7.5).powi(2)).exp();
            gun_trans[0] -= 0.010 * seat_bump;
            gun_trans[1] += 0.008 * seat_bump;
            gun_rot[2] -= 2.4 * seat_bump;

            left_hand_tex = if u < 0.32 {
                lerp3(
                    [12.20, 7.10, 12.35],
                    [11.35, 8.05, 11.75],
                    smoothstep(u / 0.32),
                )
            } else if u < 0.70 {
                lerp3(
                    [11.35, 8.05, 11.75],
                    [10.85, 8.35, 11.35],
                    smoothstep((u - 0.32) / 0.38),
                )
            } else {
                lerp3(
                    [10.85, 8.35, 11.35],
                    [12.20, 7.10, 12.35],
                    smoothstep((u - 0.70) / 0.30),
                )
            };
            left_hand_rot = [122.0, -12.0 + 4.0 * seat_bump, 24.0];
        }
        ActionAnim::ReloadEndNorm { elapsed } => {
            let dur = (RELOAD_CLOSE_NORM_TICKS as f32) * 0.05;
            let p = 1.0 - smoothstep((elapsed / dur).clamp(0.0, 1.0));
            apply_reload_cradle(&mut gun_trans, &mut gun_rot, p);
            left_hand_tex = lerp3(base_left_tex, [12.10, 7.20, 12.20], p);
            left_hand_rot = lerp3(base_left_rot, [122.0, -12.0, 24.0], p);
        }
        ActionAnim::ReloadEndEmpty { elapsed, .. } => {
            let dur = (RELOAD_CLOSE_EMPTY_TICKS as f32) * 0.05;
            let settle = 1.0 - smoothstep((elapsed / 0.28).clamp(0.0, 1.0));
            apply_reload_cradle(&mut gun_trans, &mut gun_rot, settle);
            left_hand_tex = lerp3(base_left_tex, [12.10, 7.20, 12.20], settle);
            left_hand_rot = lerp3(base_left_rot, [122.0, -12.0, 24.0], settle);

            if elapsed >= 0.15 {
                let lt = ((elapsed - 0.15) / (dur - 0.17)).clamp(0.0, 1.0);
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
                gun_trans[2] += 0.024 * lever_curve;
                right_hand_tex = [
                    base_right_tex[0],
                    base_right_tex[1] - 2.25 * lever_curve,
                    base_right_tex[2] - 2.05 * lever_curve,
                ];
                right_hand_rot[0] = base_right_rot[0] + 17.0 * lever_curve;
            }
        }
        ActionAnim::Inspect { elapsed } => {
            let u = (elapsed / INSPECT_DURATION_SECS).clamp(0.0, 1.0);
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
                let drift = ((u - 0.16) / 0.26 * PI).sin();
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
                let drift = ((u - 0.58) / 0.24 * PI).sin();
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
            left_hand_tex = lerp3(base_left_tex, [8.55, 8.15, 5.10], l_shift);
            left_hand_rot = lerp3(base_left_rot, [120.0, -14.0, 16.0], l_shift);
        }
    }

    WeaponPose {
        gun_trans,
        gun_rot,
        lever_deg,
        hammer_deg,
        trigger_deg,
        gate_deg,
        show_cartridge,
        cartridge_tex,
        cartridge_rot,
        show_casing,
        casing_tex,
        casing_rot,
        right_hand_tex,
        right_hand_rot,
        left_hand_tex,
        left_hand_rot,
    }
}

fn apply_reload_cradle(gun_trans: &mut [f32; 3], gun_rot: &mut [f32; 3], weight: f32) {
    gun_trans[0] -= 0.072 * weight;
    gun_trans[1] -= 0.044 * weight;
    gun_trans[2] += 0.044 * weight;
    gun_rot[0] += 6.8 * weight;
    gun_rot[1] += 11.5 * weight;
    gun_rot[2] -= 20.5 * weight;
}

pub(crate) fn draw_gun_space_arm(
    draw: &mut dyn BlockEntityDrawContext,
    limb: &LimbModel,
    skin: &ResolvedSkin,
    is_right_arm: bool,
    fist_tex: [f32; 3],
    rot_deg: [f32; 3],
) {
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
    limb.draw(draw, skin);
    draw.pop();
}

/// Draws the 5-part articulated Winchester 1873 (`body` + `lever` + `hammer` + `trigger` + `gate`)
/// inside `[0..1]` unit-block rifle model space (`1` unit = `16` JSON texels) for 1st-person
/// `render_hand`. Skips `push` / `rotate_about` / `pop` WASM calls for parts at rest (`0°`).
pub(crate) fn draw_rifle_assembly(
    draw: &mut dyn BlockEntityDrawContext,
    models: RifleModels,
    lever_deg: f32,
    hammer_deg: f32,
    trigger_deg: f32,
    gate_deg: f32,
) -> bool {
    if draw.model(models.body, 0xFF_FF_FF_FF).is_err() {
        return false;
    }

    if lever_deg.abs() < 1e-4 {
        let _ = draw.model(models.lever, 0xFF_FF_FF_FF);
    } else {
        draw.push();
        draw.rotate_about([1.0, 0.0, 0.0], lever_deg, tex_to_unit(LEVER_PIVOT_TEX));
        let _ = draw.model(models.lever, 0xFF_FF_FF_FF);
        draw.pop();
    }

    if hammer_deg.abs() < 1e-4 {
        let _ = draw.model(models.hammer, 0xFF_FF_FF_FF);
    } else {
        draw.push();
        draw.rotate_about([1.0, 0.0, 0.0], hammer_deg, tex_to_unit(HAMMER_PIVOT_TEX));
        let _ = draw.model(models.hammer, 0xFF_FF_FF_FF);
        draw.pop();
    }

    if trigger_deg.abs() < 1e-4 {
        let _ = draw.model(models.trigger, 0xFF_FF_FF_FF);
    } else {
        draw.push();
        draw.rotate_about([1.0, 0.0, 0.0], trigger_deg, tex_to_unit(TRIGGER_PIVOT_TEX));
        let _ = draw.model(models.trigger, 0xFF_FF_FF_FF);
        draw.pop();
    }

    if gate_deg.abs() < 1e-4 {
        let _ = draw.model(models.gate, 0xFF_FF_FF_FF);
    } else {
        draw.push();
        draw.rotate_about([0.0, 1.0, 0.0], gate_deg, tex_to_unit(GATE_PIVOT_TEX));
        let _ = draw.model(models.gate, 0xFF_FF_FF_FF);
        draw.pop();
    }

    true
}

/// Lightweight 3rd-person / multiplayer back-sling assembly (`247` faces vs `2,016`, `8.2×` lighter)
/// preserving exact silhouette, wood/steel UVs, lever loop, sights, and cocked/uncocked hammer pose.
pub(crate) fn draw_back_sling_assembly(
    draw: &mut dyn BlockEntityDrawContext,
    models: RifleModels,
    hammer_deg: f32,
) -> bool {
    if draw.model(models.body_lod, 0xFF_FF_FF_FF).is_err() {
        return false;
    }
    if hammer_deg.abs() < 1e-4 {
        let _ = draw.model(models.hammer, 0xFF_FF_FF_FF);
    } else {
        draw.push();
        draw.rotate_about([1.0, 0.0, 0.0], hammer_deg, tex_to_unit(HAMMER_PIVOT_TEX));
        let _ = draw.model(models.hammer, 0xFF_FF_FF_FF);
        draw.pop();
    }
    true
}

/// Returns `(loaded_rounds, chest_armored)` when the local player has the Winchester 1873
/// stowed in their off-hand (`Hand::Off`).
pub(crate) fn local_offhand_rifle_status(host: &dyn ClientHost) -> Option<(u32, bool)> {
    let rifle = RIFLE_KEY.get().copied()?;
    let cam = host.camera_entity()?;
    let liv = host.living(cam)?;
    let off = liv.hand_item(host, Hand::Off)?;
    if off.item() != rifle {
        return None;
    }
    let chest_armored = liv.equipment(host, EquipmentSlot::Chest).is_some();
    Some((rounds(&off), chest_armored))
}

/// Converts `[0..16]` JSON texels into right-hand view space after `RIFLE_DISPLAY`.
pub(crate) fn model_texel_to_hand(tex: [f32; 3]) -> [f32; 3] {
    let k = RIFLE_DISPLAY_SCALE / 16.0;
    [
        RIFLE_DISPLAY_TRANS[0] + (tex[0] - 8.0) * k,
        RIFLE_DISPLAY_TRANS[1] + (tex[1] - 8.0) * k,
        RIFLE_DISPLAY_TRANS[2] + (tex[2] - 8.0) * k,
    ]
}

/// Evaluates any `[0..16]` rifle JSON texel point (`MUZZLE_TEXELS`, `EJECTOR_HANDOFF_TEXELS`, …)
/// through the exact `render_hand` transform stack (`RIFLE_DISPLAY` -> `HAND_BASE` -> `gun_rot`
/// -> `gun_trans`) in 1st-person hand camera space.
pub(crate) fn compute_gun_texel_view_pos(tex: [f32; 3], pose: &WeaponPose, equip: f32) -> [f32; 3] {
    let local = model_texel_to_hand(tex);
    let mut x = HAND_BASE[0] + local[0];
    let mut y = HAND_BASE[1] - equip * 0.6 + local[1];
    let mut z = HAND_BASE[2] + local[2];

    let (sz, cz) = pose.gun_rot[2].to_radians().sin_cos();
    (x, y) = (x * cz - y * sz, x * sz + y * cz);
    let (sy, cy) = pose.gun_rot[1].to_radians().sin_cos();
    (x, z) = (x * cy + z * sy, -x * sy + z * cy);
    let (sx, cx) = pose.gun_rot[0].to_radians().sin_cos();
    (y, z) = (y * cx - z * sx, y * sx + z * cx);

    [
        x + pose.gun_trans[0],
        y + pose.gun_trans[1],
        z + pose.gun_trans[2],
    ]
}

/// Builds `ACTION_FIRE` with the exact screen-ray-matched world muzzle offset (`mm`) so muzzle
/// smoke & flash appear right on `boca_cano` in hipfire, mid-ADS, full ADS, and `F5` 3rd person
/// across any Minecraft FOV setting.
pub(crate) fn build_fire_input() -> RifleInput {
    let aim_pct = (aim_progress().clamp(0.0, 1.0) * 100.0).round() as i32;
    let (right_m, up_m, fwd_m) = if holding_rifle_recently() {
        let mx = f32::from_bits(LAST_MUZZLE_VIEW_X.load(Ordering::Relaxed));
        let my = f32::from_bits(LAST_MUZZLE_VIEW_Y.load(Ordering::Relaxed));
        let mz = f32::from_bits(LAST_MUZZLE_VIEW_Z.load(Ordering::Relaxed));
        let fwd_1p = (-mz).clamp(0.5, 3.0);
        let world_fov =
            f32::from_bits(LAST_WORLD_FOV_BITS.load(Ordering::Relaxed)).clamp(20.0, 150.0);
        // Horizon's 1st-person hand pass uses a 70° vertical FOV (`tan(35°)`), while world
        // particles project under `world_fov`. Scaling `(mx, my)` by `fov_ratio * (1.35 / fwd_1p)`
        // keeps exact 0-pixel ray alignment with `boca_cano` on screen at `1.35` blocks barrel reach.
        let fov_ratio = (world_fov.to_radians() * 0.5).tan() / (35.0_f32.to_radians()).tan();
        let target_fwd = 1.35_f32;
        let ray_scale = target_fwd / fwd_1p;
        (
            (mx * fov_ratio * ray_scale).clamp(-1.1, 1.1),
            (my * fov_ratio * ray_scale).clamp(-1.1, 1.1),
            target_fwd,
        )
    } else {
        // 3rd-person (`F5`) held barrel tip offset relative to eye basis.
        (0.28_f32, -0.14_f32, 1.35_f32)
    };

    RifleInput {
        action: VarInt(ACTION_FIRE),
        muzzle_right_mm: VarInt((right_m * 1000.0).round() as i32),
        muzzle_up_mm: VarInt((up_m * 1000.0).round() as i32),
        muzzle_fwd_mm: VarInt((fwd_m * 1000.0).round() as i32),
        aim_pct: VarInt(aim_pct),
    }
}

pub(crate) const fn tex_to_unit(p: [f32; 3]) -> [f32; 3] {
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

pub(crate) fn cam_recoil_curve(t: f32) -> (f32, f32) {
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

pub(crate) fn recoil_envelope(t: f32) -> f32 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_order_spring_converges_across_frame_rates() {
        for dt in [1.0 / 240.0_f32, 1.0 / 60.0, 1.0 / 25.0] {
            let mut s = SecondOrder::new(3.8, 0.88, 1.05, 0.0);
            let steps = (1.5 / dt).ceil() as usize;
            for _ in 0..steps {
                let y = s.step(dt, 1.0);
                assert!(y.is_finite());
            }
            assert!((s.y - 1.0).abs() < 1e-2, "dt={dt} settled at {}", s.y);
        }
    }

    #[test]
    fn recoil_curves_peak_and_recover_cleanly() {
        assert_eq!(cam_recoil_curve(0.0), (0.0, 0.0));
        assert_eq!(cam_recoil_curve(0.12), (1.0, 1.0));
        let (end_p, end_y) = cam_recoil_curve(0.45);
        assert!((end_p - (1.0 - 1.0 / 1.75)).abs() < 1e-5);
        assert!((end_y - 0.5).abs() < 1e-5);

        assert_eq!(recoil_envelope(-0.01), 0.0);
        assert!((recoil_envelope(0.035) - 1.0).abs() < 1e-5);
        assert_eq!(recoil_envelope(0.25), 0.0);
    }

    #[test]
    fn ads_sight_line_aligns_with_camera_center() {
        let pose = evaluate_weapon_pose(
            1.0,
            0.0,
            0.0,
            0.0,
            1.0,
            1.0,
            1.0,
            0.0,
            0.0,
            0.0,
            1.0,
            false,
            ActionAnim::None,
        );
        let sight_view = compute_gun_texel_view_pos([8.139, 12.684, 0.0], &pose, 0.0);
        assert!(
            sight_view[0].abs() < 0.004 && sight_view[1].abs() < 0.004,
            "ADS sight line drifted from center: {sight_view:?}"
        );
        assert!(sight_view[2] < -0.5);
    }
}
