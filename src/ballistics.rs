//! `.44-40` terminal ballistics: sub-tick 3D ray-AABB headshot geometry,
//! smooth long-range velocity/damage falloff (`> 40 m`), and short-lived
//! `projectile_impact` -> `living_damage_pre` impact handoff.

use std::sync::Mutex;

/// Precision cranial impact multiplier (`14.0 * 1.5 = 21.0` HP = `10.5` hearts).
pub(crate) const HEADSHOT_MULTIPLIER: f32 = 1.5;
/// Full-power `.44-40` flat-damage plateau (`0 ..= 40` blocks).
pub(crate) const FALLOFF_START_METERS: f64 = 40.0;
/// Extreme frontier range where smooth Hermite falloff levels off (`110` blocks).
pub(crate) const FALLOFF_END_METERS: f64 = 110.0;
/// Minimum retained `.44-40` damage factor at `>= 110` blocks (`72%`).
pub(crate) const FALLOFF_FLOOR_MULTIPLIER: f32 = 0.72;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BallisticEvaluation {
    pub(crate) is_headshot: bool,
    pub(crate) total_multiplier: f32,
}

#[derive(Clone, Copy, Debug)]
struct PendingBulletImpact {
    owner_id: i32,
    target_id: i32,
    eval: BallisticEvaluation,
    age_ticks: u8,
}

static PENDING_IMPACTS: Mutex<Vec<PendingBulletImpact>> = Mutex::new(Vec::new());

pub(crate) fn clear_all_pending_impacts() {
    if let Ok(mut list) = PENDING_IMPACTS.lock() {
        list.clear();
    }
}

pub(crate) fn tick_pending_impacts() {
    if let Ok(mut list) = PENDING_IMPACTS.lock() {
        list.retain_mut(|entry| {
            entry.age_ticks = entry.age_ticks.saturating_add(1);
            entry.age_ticks <= 4
        });
    }
}

pub(crate) fn record_pending_impact(owner_id: i32, target_id: i32, eval: BallisticEvaluation) {
    if let Ok(mut list) = PENDING_IMPACTS.lock() {
        if list.len() >= 64 {
            list.remove(0);
        }
        list.push(PendingBulletImpact {
            owner_id,
            target_id,
            eval,
            age_ticks: 0,
        });
    }
}

pub(crate) fn take_pending_impact(owner_id: i32, target_id: i32) -> Option<BallisticEvaluation> {
    let mut list = PENDING_IMPACTS.lock().ok()?;
    let idx = list
        .iter()
        .rposition(|e| e.owner_id == owner_id && e.target_id == target_id)?;
    Some(list.swap_remove(idx).eval)
}

/// Smooth Hermite (`smoothstep`) distance damage factor:
/// - `0 ..= 40 m`: `1.00×` (`14.0` body / `21.0` headshot)
/// - `40 .. 110 m`: smooth roll-off from `1.00×` down to `0.72×`
///   (at `50 m`, headshot is still `20.7` HP — rewarding long-range ADS headshots!)
/// - `>= 110 m`: `0.72×` (`10.1` body / `15.1` headshot)
pub(crate) fn distance_damage_multiplier(distance_m: f64) -> f32 {
    if !distance_m.is_finite() || distance_m <= FALLOFF_START_METERS {
        return 1.0;
    }
    if distance_m >= FALLOFF_END_METERS {
        return FALLOFF_FLOOR_MULTIPLIER;
    }
    let t = ((distance_m - FALLOFF_START_METERS) / (FALLOFF_END_METERS - FALLOFF_START_METERS))
        .clamp(0.0, 1.0) as f32;
    let smooth = t * t * (3.0 - 2.0 * t);
    1.0 - smooth * (1.0 - FALLOFF_FLOOR_MULTIPLIER)
}

/// Evaluates combined distance falloff + `1.5×` headshot terminal multiplier.
pub(crate) fn evaluate_ballistics(distance_m: f64, is_headshot: bool) -> BallisticEvaluation {
    let distance_factor = distance_damage_multiplier(distance_m);
    let head_factor = if is_headshot {
        HEADSHOT_MULTIPLIER
    } else {
        1.0
    };
    BallisticEvaluation {
        is_headshot,
        total_multiplier: distance_factor * head_factor,
    }
}

/// 3D Kay-Kajiya slab ray-AABB entry point (`t >= 0` or origin inside box).
pub fn ray_aabb_entry_point(
    origin: [f64; 3],
    dir: [f64; 3],
    min: [f64; 3],
    max: [f64; 3],
) -> Option<[f64; 3]> {
    let mut t_min = 0.0_f64;
    let mut t_max = f64::INFINITY;

    for axis in 0..3 {
        let o = origin[axis];
        let d = dir[axis];
        let b_min = min[axis];
        let b_max = max[axis];
        if d.abs() < 1e-9 {
            if o < b_min || o > b_max {
                return None;
            }
        } else {
            let inv = 1.0 / d;
            let mut t1 = (b_min - o) * inv;
            let mut t2 = (b_max - o) * inv;
            if t1 > t2 {
                std::mem::swap(&mut t1, &mut t2);
            }
            t_min = t_min.max(t1);
            t_max = t_max.min(t2);
            if t_min > t_max {
                return None;
            }
        }
    }

    Some([
        origin[0] + dir[0] * t_min,
        origin[1] + dir[1] * t_min,
        origin[2] + dir[2] * t_min,
    ])
}

/// Anatomical lower Y boundary of a living entity's cranial zone, or `None` for
/// tiny/flat critters (`height < 0.65`) that have no distinct upper head hitbox.
pub fn anatomical_head_floor_y(
    min_y: f64,
    max_y: f64,
    eye_height: Option<f64>,
) -> Option<f64> {
    let height = max_y - min_y;
    if !height.is_finite() || height < 0.65 {
        return None;
    }
    if height >= 1.35 {
        // Humanoid & tall mobs (Player 1.8, Zombie 1.95, Skeleton 1.99, Creeper 1.7,
        // Enderman 2.9): top ~28% skull box anchored seamlessly around eye_height.
        let top_band = max_y - (height * 0.28).clamp(0.38, 0.62);
        let floor = match eye_height {
            Some(eh) if eh.is_finite() && eh > height * 0.60 && eh < height => {
                let eye_floor = min_y + eh - (height * 0.13).clamp(0.16, 0.28);
                top_band.min(eye_floor)
            }
            _ => top_band,
        };
        Some(floor.clamp(min_y + height * 0.55, max_y - 0.15))
    } else {
        // Compact upright mobs (e.g. Baby Zombie 0.975, Spider 0.9): upper 30% head/eye band.
        Some(max_y - height * 0.30)
    }
}

/// Sub-tick 3D ray-AABB headshot test for fast `15.0 blocks/tick` (`300 m/s`) rounds.
/// Traces from `ray_origin` along `ray_dir` into `aabb` (with vanilla's `0.3`-block
/// projectile collision inflation fallback) and checks whether the entry point lands
/// inside the target's cranial band.
pub fn is_ray_headshot_on_box(
    ray_origin: [f64; 3],
    ray_dir: [f64; 3],
    fallback_hit_pos: [f64; 3],
    aabb: [[f64; 3]; 2],
    eye_height: Option<f64>,
) -> bool {
    let [min, max] = aabb;
    let Some(head_floor_y) = anatomical_head_floor_y(min[1], max[1], eye_height) else {
        return false;
    };

    let dir_len_sq =
        ray_dir[0] * ray_dir[0] + ray_dir[1] * ray_dir[1] + ray_dir[2] * ray_dir[2];

    let impact_y = if dir_len_sq > 1e-8 {
        if let Some(pt) = ray_aabb_entry_point(ray_origin, ray_dir, min, max) {
            pt[1]
        } else {
            // Vanilla ProjectileUtil::getEntityHitResult inflates entity AABBs by 0.3 blocks.
            let inflated_min = [min[0] - 0.32, min[1] - 0.18, min[2] - 0.32];
            let inflated_max = [max[0] + 0.32, max[1] + 0.18, max[2] + 0.32];
            if let Some(pt) =
                ray_aabb_entry_point(ray_origin, ray_dir, inflated_min, inflated_max)
            {
                pt[1].clamp(min[1], max[1])
            } else {
                fallback_hit_pos[1]
            }
        }
    } else {
        fallback_hit_pos[1]
    };

    impact_y >= head_floor_y && impact_y <= max[1] + 0.20
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE_BULLET_DAMAGE: f32 = 14.0;

    #[test]
    fn flat_damage_plateau_within_forty_meters() {
        assert!((distance_damage_multiplier(0.0) - 1.0).abs() < 1e-6);
        assert!((distance_damage_multiplier(25.0) - 1.0).abs() < 1e-6);
        assert!((distance_damage_multiplier(40.0) - 1.0).abs() < 1e-6);

        let body_close = BASE_BULLET_DAMAGE * evaluate_ballistics(20.0, false).total_multiplier;
        let head_close = BASE_BULLET_DAMAGE * evaluate_ballistics(20.0, true).total_multiplier;
        assert!((body_close - 14.0).abs() < 1e-4);
        assert!((head_close - 21.0).abs() < 1e-4);
    }

    #[test]
    fn smooth_long_range_falloff_rewards_precision_headshots() {
        let m50 = evaluate_ballistics(50.0, true);
        let d50 = distance_damage_multiplier(50.0);
        let d75 = distance_damage_multiplier(75.0);
        let d110 = distance_damage_multiplier(110.0);
        let d200 = distance_damage_multiplier(200.0);

        // At 50m ("Long Shot" advancement threshold), a headshot still deals >20 HP (1-shot Skeleton!).
        assert!(BASE_BULLET_DAMAGE * m50.total_multiplier > 20.0);
        // Monotonically decreasing between 40m and 110m, then clamped cleanly at 0.72x.
        assert!(d50 > d75);
        assert!(d75 > d110);
        assert!((d110 - FALLOFF_FLOOR_MULTIPLIER).abs() < 1e-6);
        assert!((d200 - FALLOFF_FLOOR_MULTIPLIER).abs() < 1e-6);
    }

    #[test]
    fn sub_tick_fast_bullet_distinguishes_skull_from_torso_on_skeleton() {
        // Skeleton standing at Z = 20.0, feet at Y = 64.0, height = 1.99, eye_height = 1.74.
        let skeleton_aabb = [[-0.3, 64.0, 19.7], [0.3, 65.99, 20.3]];
        let eye_h = Some(1.74);

        // 15 blocks/tick round starting 11 blocks upstream aimed at eye level (Y = 65.72): HEADSHOT!
        assert!(is_ray_headshot_on_box(
            [0.0, 65.72, 9.0],
            [0.0, 0.0, 15.0],
            [0.0, 65.0, 19.7],
            skeleton_aabb,
            eye_h,
        ));

        // Same 15 blocks/tick round aimed center-mass at chest (Y = 65.05): BODY SHOT!
        assert!(!is_ray_headshot_on_box(
            [0.0, 65.05, 9.0],
            [0.0, 0.0, 15.0],
            [0.0, 65.05, 19.7],
            skeleton_aabb,
            eye_h,
        ));
    }

    #[test]
    fn enderman_tall_hitbox_and_tiny_silverfish_exclusion() {
        let enderman_aabb = [[-0.3, 70.0, 10.0], [0.3, 72.9, 10.6]];
        // Aiming at Enderman's chest (Y = 71.7, 1.7m above feet) is NOT a headshot:
        assert!(!is_ray_headshot_on_box(
            [0.0, 71.7, 0.0],
            [0.0, 0.0, 15.0],
            [0.0, 71.7, 10.0],
            enderman_aabb,
            Some(2.55),
        ));
        // Aiming at Enderman's eyes (Y = 72.55) IS a headshot:
        assert!(is_ray_headshot_on_box(
            [0.0, 72.55, 0.0],
            [0.0, 0.0, 15.0],
            [0.0, 72.55, 10.0],
            enderman_aabb,
            Some(2.55),
        ));

        // Silverfish (height = 0.3) never triggers cranial headshots:
        let silverfish_aabb = [[-0.2, 64.0, 5.0], [0.2, 64.3, 5.4]];
        assert!(!is_ray_headshot_on_box(
            [0.0, 64.28, 0.0],
            [0.0, 0.0, 15.0],
            [0.0, 64.28, 5.0],
            silverfish_aabb,
            Some(0.13),
        ));
    }
}
