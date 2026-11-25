//! 1st-person player arm meshes (`wide` `4×12×4` + `slim` `3×12×4` + `0.25` 3D sleeves).

use std::sync::Mutex;

use mod_api::{
    BlockEntityDrawContext, ClientHost, CubeDeformation, CubeListBuilder, LayerDefinition,
    MeshDefinition, ModelPart, PartPose,
};

use crate::constants::DEFAULT_STEVE_WIDE;
use crate::skin::ResolvedSkin;

/// Caches `(primary, fallback) -> active_sprite` so `draw.model_part` never serializes an entire
/// `ModelPart` across WASM twice per arm when `skin.primary` is absent from the sprite table.
static CACHED_SKIN_SPRITE: Mutex<Option<(String, String, String)>> = Mutex::new(None);

pub(crate) fn invalidate_skin_sprite_cache() {
    *CACHED_SKIN_SPRITE.lock().unwrap() = None;
}

pub(crate) struct LimbModel {
    limb: ModelPart,
    detached_sleeve: Option<ModelPart>,
}

pub(crate) struct ArmPair {
    pub(crate) right: LimbModel,
    pub(crate) left: LimbModel,
}

pub(crate) struct PlayerArms {
    pub(crate) wide: ArmPair,
    pub(crate) slim: ArmPair,
}

impl LimbModel {
    fn extract(root: &ModelPart, limb_name: &str, sleeve_name: &str) -> Option<Self> {
        let mut limb = root.get_child(limb_name)?.clone();
        limb.load_pose(PartPose::ZERO);
        let detached_sleeve = if limb.get_child(sleeve_name).is_some() {
            None
        } else {
            root.get_child(sleeve_name).cloned().map(|mut s| {
                s.load_pose(PartPose::ZERO);
                s
            })
        };
        Some(Self {
            limb,
            detached_sleeve,
        })
    }

    fn draw_part_with_skin(
        draw: &mut dyn BlockEntityDrawContext,
        part: &ModelPart,
        skin: &ResolvedSkin,
    ) {
        let cached = {
            let guard = CACHED_SKIN_SPRITE.lock().unwrap();
            guard
                .as_ref()
                .filter(|(p, f, _)| p == &skin.primary && f == &skin.fallback)
                .map(|(_, _, active)| active.clone())
        };
        if let Some(active) = cached
            && draw.model_part(part, &active, 0xFF_FFFF)
        {
            return;
        }

        let resolved = if draw.model_part(part, &skin.primary, 0xFF_FFFF) {
            skin.primary.clone()
        } else if draw.model_part(part, &skin.fallback, 0xFF_FFFF) {
            skin.fallback.clone()
        } else {
            let _ = draw.model_part(part, DEFAULT_STEVE_WIDE, 0xFF_FFFF);
            DEFAULT_STEVE_WIDE.to_string()
        };
        *CACHED_SKIN_SPRITE.lock().unwrap() =
            Some((skin.primary.clone(), skin.fallback.clone(), resolved));
    }

    pub(crate) fn draw(&self, draw: &mut dyn BlockEntityDrawContext, skin: &ResolvedSkin) {
        Self::draw_part_with_skin(draw, &self.limb, skin);
        if let Some(sleeve) = &self.detached_sleeve {
            Self::draw_part_with_skin(draw, sleeve, skin);
        }
    }
}

impl ArmPair {
    fn from_baked_root(root: &ModelPart) -> Option<Self> {
        Some(Self {
            right: LimbModel::extract(root, "right_arm", "right_sleeve")?,
            left: LimbModel::extract(root, "left_arm", "left_sleeve")?,
        })
    }

    fn procedural(slim: bool) -> Self {
        let width = if slim { 3.0 } else { 4.0 };
        let right_origin_x = if slim { -2.0 } else { -3.0 };
        let build_limb = |origin_x: f32, arm_uv: (i32, i32), sleeve_uv: (i32, i32)| -> LimbModel {
            let mut mesh = MeshDefinition::new();
            let root = mesh.root();
            let arm = mesh.add_or_replace_child(
                root,
                "arm",
                CubeListBuilder::create()
                    .tex_offs(arm_uv.0, arm_uv.1)
                    .add_box([origin_x, -2.0, -2.0], [width, 12.0, 4.0]),
                PartPose::ZERO,
            );
            mesh.add_or_replace_child(
                arm,
                "sleeve",
                CubeListBuilder::create()
                    .tex_offs(sleeve_uv.0, sleeve_uv.1)
                    .add_box_with(
                        [origin_x, -2.0, -2.0],
                        [width, 12.0, 4.0],
                        CubeDeformation::new(0.25),
                    ),
                PartPose::ZERO,
            );
            let baked = LayerDefinition::create(mesh, 64, 64).bake_root();
            let mut limb = baked.get_child("arm").cloned().unwrap_or(baked);
            limb.load_pose(PartPose::ZERO);
            LimbModel {
                limb,
                detached_sleeve: None,
            }
        };
        Self {
            right: build_limb(right_origin_x, (40, 16), (40, 32)),
            left: build_limb(-1.0, (32, 48), (48, 48)),
        }
    }
}

impl PlayerArms {
    pub(crate) fn bake(host: &dyn ClientHost) -> Self {
        let wide = host
            .bake_layer("minecraft:player#main")
            .and_then(|r| ArmPair::from_baked_root(&r))
            .unwrap_or_else(|| ArmPair::procedural(false));
        let slim = host
            .bake_layer("minecraft:player_slim#main")
            .and_then(|r| ArmPair::from_baked_root(&r))
            .unwrap_or_else(|| ArmPair::procedural(true));
        Self { wide, slim }
    }

    pub(crate) fn procedural_both() -> Self {
        Self {
            wide: ArmPair::procedural(false),
            slim: ArmPair::procedural(true),
        }
    }
}
