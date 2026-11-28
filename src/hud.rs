//! Western brass HUD ammo counter, optional crimson-gold headshot hitmarker, ADS vignette &
//! crosshair suppression, and rich `.44-40` / Winchester 1873 item tooltips.

use std::sync::atomic::Ordering;

use mod_api::text::TextContent;
use mod_api::{
    ClientHost, Flow, GuiDraw, GuiLayerPreEvent, GuiRenderPostEvent, Hand, ItemTooltipEvent, Text,
};

use crate::constants::*;
use crate::server::rounds;
use crate::settings::client_settings;
use crate::{
    aim_progress, client_anim, holding_rifle_recently, AIMING, CARTRIDGE_KEY, CLIENT_RELOADING,
    HEADSHOT_HITMARKER, HITMARKER_TIMER_BITS, LOCAL_RESERVE_AMMO, RIFLE_KEY,
};

pub(crate) fn count_local_reserve_ammo(host: &dyn ClientHost) -> u32 {
    let Some(cartridge) = CARTRIDGE_KEY.get().copied() else {
        return 0;
    };
    let Some(player) = host.local_player() else {
        return 0;
    };
    if player.is_creative(host).unwrap_or(false) {
        return u32::MAX;
    }
    let mut total: u32 = 0;
    if let Some(items) = player.inventory(host) {
        for st in items {
            if st.item() == cartridge {
                total = total.saturating_add(st.count());
            }
        }
    }
    if let Some(liv) = host.living(player.entity())
        && let Some(off) = liv.hand_item(host, Hand::Off)
        && off.item() == cartridge
    {
        total = total.saturating_add(off.count());
    }
    total
}

fn styled_text(mut text: Text, color_hex: &str, italic: bool) -> Text {
    text.style.color = Some(color_hex.into());
    text.style.italic = Some(italic);
    text
}

fn pretty_key_fallback(key_id: &str, default_label: &str) -> String {
    if let Some(rest) = key_id.strip_prefix("key.keyboard.") {
        if rest.len() == 1 {
            return rest.to_ascii_uppercase();
        }
        return rest.replace('.', " ").to_ascii_uppercase();
    }
    if let Some(rest) = key_id.strip_prefix("key.mouse.") {
        return format!("M:{}", rest.to_ascii_uppercase());
    }
    default_label.into()
}

fn mod_key_component(host: &dyn ClientHost, mapping: &str, fallback_label: &str) -> Text {
    let base = match host.mod_key(MOD_ID, mapping) {
        Some((key_id, _)) if !key_id.is_empty() && key_id != "key.keyboard.unknown" => {
            Text::new(TextContent::Translatable {
                key: key_id.clone(),
                fallback: Some(pretty_key_fallback(&key_id, fallback_label)),
                arguments: Vec::new(),
            })
        }
        _ => Text::literal(fallback_label),
    };
    styled_text(base, "#F2D56F", false)
}

fn vanilla_key_component(mapping: &str) -> Text {
    styled_text(
        Text::new(TextContent::Keybind(mapping.into())),
        "#F2D56F",
        false,
    )
}

fn text_contains_durability(text: &Text) -> bool {
    if let TextContent::Translatable { key, .. } = &text.content
        && key == "item.durability"
    {
        return true;
    }
    text.children.iter().any(text_contains_durability)
}

/// Draws a crisp pixel-perfect mini `.44-40` brass/copper cartridge icon (`3×9` GUI px) inside
/// the HUD reserve counter box with zero texture state switches.
fn draw_hud_mini_cartridge(event: &mut GuiRenderPostEvent<'_>, x: f32, y: f32) {
    // Copper jacketed soft-lead nose (top 3px)
    event.batch.fill([x + 0.5, y, 2.0, 1.0], 0xFF_F2_AD_72);
    event.batch.fill([x, y + 1.0, 3.0, 2.0], 0xFF_DB_89_48);
    event.batch.fill([x, y + 1.0, 1.0, 2.0], 0xFF_F2_AD_72);
    // Polished .44-40 bottleneck brass case (middle 5px)
    event.batch.fill([x, y + 3.0, 3.0, 5.0], 0xFF_BF_95_28);
    event.batch.fill([x + 0.5, y + 3.0, 1.2, 5.0], 0xFF_F2_D5_6F);
    // Rimmed centerfire base (bottom 1px, slightly wider)
    event.batch.fill([x - 0.5, y + 8.0, 4.0, 1.0], 0xFF_99_73_1C);
    event.batch.fill([x, y + 8.0, 2.0, 1.0], 0xFF_E6_C0_4C);
}

pub(crate) fn render_item_tooltip(host: &dyn ClientHost, event: &mut ItemTooltipEvent<'_>) {
    let rifle = RIFLE_KEY.get().copied();
    let cartridge = CARTRIDGE_KEY.get().copied();
    let item = event.stack.item();
    let is_rifle = rifle == Some(item);
    let is_cartridge = cartridge == Some(item);
    if !is_rifle && !is_cartridge {
        return;
    }

    // Always hide Vanilla's internal `"item.durability"` line on the Winchester (`F3+H`),
    // because stack `damage` encodes loaded `.44-40` rounds (`0..=8`), not wear & tear.
    if is_rifle {
        event.lines.retain(|(line, _)| !text_contains_durability(line));
    }

    if !client_settings().hud_rich_tooltips {
        return;
    }

    let insert_at = if event.lines.is_empty() { 0 } else { 1 };
    let mut extra: Vec<(Text, u32)> = Vec::with_capacity(4);

    if is_rifle {
        let loaded = rounds(event.stack);
        let reserve = LOCAL_RESERVE_AMMO.load(Ordering::Relaxed);
        let loaded_color = if loaded == 0 { "#FF6E59" } else { "#F2D56F" };
        let loaded_arg = styled_text(Text::literal(loaded.to_string()), loaded_color, false);
        let cap_arg = styled_text(Text::literal(TUBE_CAPACITY.to_string()), "#E6D6B8", false);
        let reserve_arg = if reserve == u32::MAX {
            styled_text(Text::literal("∞"), "#FFE082", false)
        } else if reserve == 0 {
            styled_text(Text::literal("0"), "#D96B59", false)
        } else {
            styled_text(Text::literal(reserve.to_string()), "#E6D6B8", false)
        };

        extra.push((
            styled_text(
                Text::translate("tooltip.just_a_rifle.winchester.subtitle", Vec::new()),
                "#B8A88A",
                true,
            ),
            u32::MAX,
        ));
        extra.push((
            styled_text(
                Text::translate(
                    "tooltip.just_a_rifle.winchester.ammo",
                    vec![loaded_arg, cap_arg, reserve_arg],
                ),
                "#C8BEA8",
                false,
            ),
            u32::MAX,
        ));
        extra.push((
            styled_text(
                Text::translate("tooltip.just_a_rifle.winchester.stats", Vec::new()),
                "#969CA6",
                false,
            ),
            u32::MAX,
        ));
        extra.push((
            styled_text(
                Text::translate(
                    "tooltip.just_a_rifle.winchester.controls",
                    vec![
                        vanilla_key_component("key.use"),
                        mod_key_component(host, RELOAD_MAPPING, "R"),
                        mod_key_component(host, INSPECT_MAPPING, "I"),
                        vanilla_key_component("key.swapOffhand"),
                    ],
                ),
                "#857E72",
                false,
            ),
            u32::MAX,
        ));
    } else if is_cartridge {
        extra.push((
            styled_text(
                Text::translate("tooltip.just_a_rifle.cartridge.caliber", Vec::new()),
                "#D9B038",
                true,
            ),
            u32::MAX,
        ));
        extra.push((
            styled_text(
                Text::translate(
                    "tooltip.just_a_rifle.cartridge.usage",
                    vec![mod_key_component(host, RELOAD_MAPPING, "R")],
                ),
                "#A89F8D",
                false,
            ),
            u32::MAX,
        ));
    }

    event.lines.splice(insert_at..insert_at, extra);
}

pub(crate) fn render_gui_layer_pre(event: &mut GuiLayerPreEvent<'_>) -> Flow {
    if client_settings().hide_crosshair_ads
        && (event.layer == "minecraft:crosshair" || event.layer == "crosshair")
        && (AIMING.load(Ordering::Relaxed)
            || (holding_rifle_recently() && aim_progress() > 0.08))
    {
        return Flow::Cancel;
    }
    Flow::Continue
}

pub(crate) fn render_gui_post(event: &mut GuiRenderPostEvent<'_>) {
    if event.gui_hidden || !holding_rifle_recently() {
        return;
    }
    let cfg = client_settings();
    let aim = aim_progress().clamp(0.0, 1.0);
    let [wf, hf] = event.batch.size();
    let (w, h) = (wf as i32, hf as i32);

    if aim >= 0.02 && cfg.ads_vignette_mul > 0.01 {
        let darken = ((1.0 - AIM_VIGNETTE) * cfg.ads_vignette_mul * aim).clamp(0.0, 0.85);
        let brightness = ((1.0 - darken) * 255.0).clamp(0.0, 255.0) as u32;
        let color = 0xFF00_0000 | (brightness << 16) | (brightness << 8) | brightness;
        let _ = event.batch.gui_paint(&[GuiDraw::Blit {
            texture: "misc/vignette".into(),
            blend: mod_api::client::hud_layout::Blend::Vignette,
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

    // Optional subtle brass hitmarker (or crimson-gold double tick on 1.5x headshots!).
    let hit_rem = f32::from_bits(HITMARKER_TIMER_BITS.load(Ordering::Relaxed));
    if cfg.hud_hitmarker && hit_rem > 0.001 {
        let is_headshot = HEADSHOT_HITMARKER.load(Ordering::Relaxed);
        let dur = if is_headshot { 0.22_f32 } else { 0.14_f32 };
        let alpha = ((hit_rem / dur).clamp(0.0, 1.0) * 235.0) as u32;
        let rgb = if is_headshot {
            0xFF_6A_4D
        } else {
            cfg.hud_accent_color & 0xFF_FFFF
        };
        let hm_color = (alpha << 24) | rgb;
        let cx = wf * 0.5;
        let cy = hf * 0.5;
        for step in 2..=5 {
            let d = step as f32;
            event.batch.fill([cx - d - 0.5, cy - d - 0.5, 1.2, 1.2], hm_color);
            event.batch.fill([cx + d - 0.7, cy - d - 0.5, 1.2, 1.2], hm_color);
            event.batch.fill([cx - d - 0.5, cy + d - 0.7, 1.2, 1.2], hm_color);
            event.batch.fill([cx + d - 0.7, cy + d - 0.7, 1.2, 1.2], hm_color);
        }
        if is_headshot {
            let gold_color = ((alpha * 4 / 5) << 24) | 0xFF_DF_78;
            for step in 6..=7 {
                let d = step as f32;
                event
                    .batch
                    .fill([cx - d - 0.5, cy - d - 0.5, 1.3, 1.3], gold_color);
                event
                    .batch
                    .fill([cx + d - 0.8, cy - d - 0.5, 1.3, 1.3], gold_color);
                event
                    .batch
                    .fill([cx - d - 0.5, cy + d - 0.8, 1.3, 1.3], gold_color);
                event
                    .batch
                    .fill([cx + d - 0.8, cy + d - 0.8, 1.3, 1.3], gold_color);
            }
        }
    }

    // Configurable western brass ammo counter (`8 / 8` + optional `.44-40` reserve with mini icon).
    let (rounds_now, reloading) = {
        let anim = client_anim().lock().unwrap();
        (
            anim.known_rounds,
            CLIENT_RELOADING.load(Ordering::Relaxed),
        )
    };
    let label = if reloading {
        format!("{rounds_now}/{TUBE_CAPACITY}+")
    } else {
        format!("{rounds_now} / {TUBE_CAPACITY}")
    };
    let mag_tw = event.batch.text_width(&label);

    let show_res = cfg.hud_show_reserve;
    let reserve = LOCAL_RESERVE_AMMO.load(Ordering::Relaxed);
    let res_label = if show_res {
        if reserve == u32::MAX {
            "∞".to_string()
        } else {
            reserve.min(9999).to_string()
        }
    } else {
        String::new()
    };
    let res_tw = if show_res {
        event.batch.text_width(&res_label)
    } else {
        0.0
    };

    let left_w = (mag_tw + 12.0).max(46.0);
    let right_w = if show_res {
        (13.0 + res_tw + 6.0).max(26.0)
    } else {
        0.0
    };
    let box_w = if show_res {
        left_w + 1.0 + right_w
    } else {
        52.0_f32.max(left_w)
    };
    let box_h = 15.0_f32;
    let Some([x0, y0]) = cfg.hud_position.layout(wf, hf, box_w, box_h) else {
        return;
    };

    let accent = cfg.hud_accent_color & 0xFF_FFFF;
    let border = if reloading {
        0xDD00_0000 | accent
    } else if rounds_now == 0 {
        0xCC_C9_4A_38
    } else {
        0xAE00_0000 | accent
    };
    event.batch.fill([x0, y0, box_w, box_h], 0x92_14_11_0E);
    event.batch.fill([x0, y0, box_w, 1.0], border);
    event.batch.fill([x0, y0 + box_h - 1.0, box_w, 1.0], border);

    let text_color = if rounds_now == 0 && !reloading {
        0xFF_FF_6E_59
    } else {
        0xFF_F5_E6_C8
    };
    let mag_area_w = if show_res { left_w } else { box_w };
    event.batch.text(
        &label,
        [x0 + (mag_area_w - mag_tw) * 0.5, y0 + 4.0],
        text_color,
        true,
    );

    if show_res {
        let div_x = x0 + left_w;
        event
            .batch
            .fill([div_x, y0 + 2.5, 1.0, box_h - 5.0], 0x6600_0000 | accent);
        draw_hud_mini_cartridge(event, div_x + 5.0, y0 + 3.0);
        let res_color = if reserve == u32::MAX {
            0xFF_FF_E0_82
        } else if reserve == 0 {
            0xFF_D9_6B_59
        } else {
            0xFF_D8_CB_B2
        };
        event
            .batch
            .text(&res_label, [div_x + 12.0, y0 + 4.0], res_color, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_key_fallback_formats_keyboard_and_mouse_bindings() {
        assert_eq!(pretty_key_fallback("key.keyboard.r", "R"), "R");
        assert_eq!(
            pretty_key_fallback("key.keyboard.left.alt", "ALT"),
            "LEFT ALT"
        );
        assert_eq!(pretty_key_fallback("key.mouse.4", "M4"), "M:4");
        assert_eq!(pretty_key_fallback("custom.unknown", "X"), "X");
    }

    #[test]
    fn durability_detector_spots_nested_vanilla_durability_lines() {
        let dur = Text::translate("item.durability", Vec::new());
        assert!(text_contains_durability(&dur));
        let mut wrapper = Text::literal("");
        wrapper.children.push(dur);
        assert!(text_contains_durability(&wrapper));
        assert!(!text_contains_durability(&Text::literal("8 / 8")));
    }
}
