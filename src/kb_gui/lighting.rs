//! Key lighting page: effect, side lights and per-key colours.
use super::{Keyboard, PALETTE};
use crate::icons as icon;
use crate::keybed::{self, Look, BADGE};
use crate::magneton::protocol::KeyLight;
use crate::magneton::state::Layer;
use crate::magneton::tables::{EFFECTS, MULTICOLOUR, MULTICOLOUR_ONLY, SINGLE_COLOUR};
use crate::theme::*;
use eframe::egui::{self, Color32, RichText, Ui};

/// LED indices of the side light bars.
const SIDE: std::ops::Range<usize> = 128..158;
/// The "Single Color" effect.
const SINGLE_COLOR_EFFECT: u8 = 6;

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

impl Keyboard {
    pub fn lighting(&mut self, ui: &mut Ui) {
        page_header(ui, "Key lighting", "Effects, side lights and per-key colours", |_| {});
        self.status_banner(ui);
        let paint = self.paint;
        let l = self.draft.lighting_state_mut();
        card(ui, icon::SPARKLE, "Effect", |ui| effect(ui, l));
        card(ui, icon::LIGHTBULB, "Side lights", |ui| side_lights(ui, l));
        let mut new_paint = None;
        card(ui, icon::PALETTE, "Per-key colours", |ui| new_paint = per_key(ui, l, paint));
        if let Some(c) = new_paint {
            self.paint = c;
        }
    }
}

fn effect(ui: &mut Ui, l: &mut Layer) {
    egui::Grid::new("kb-effects").num_columns(4).spacing([8.0, 8.0]).show(ui, |ui| {
        for (k, (name, value)) in EFFECTS.iter().enumerate() {
            let selected = l.lighting.effect == *value;
            let text = RichText::new(*name).color(if selected { ON_ACCENT } else { TEXT });
            let button = egui::Button::new(text).fill(if selected { ACCENT } else { SURFACE2 }).truncate();
            if ui.add_sized([178.0, 36.0], button).clicked() {
                l.lighting.effect = *value;
                if MULTICOLOUR_ONLY.contains(value) {
                    l.lighting.color_mode = MULTICOLOUR;
                } else if *value == SINGLE_COLOR_EFFECT {
                    l.lighting.color_mode = SINGLE_COLOUR;
                }
            }
            if k % 4 == 3 {
                ui.end_row();
            }
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        row_label(ui, "Brightness", "");
        ui.add(egui::Slider::new(&mut l.lighting.brightness, 0..=100).suffix("%"));
    });
    ui.horizontal(|ui| {
        row_label(ui, "Speed", "");
        ui.add(egui::Slider::new(&mut l.lighting.speed, 0..=100).suffix("%"));
    });
    if MULTICOLOUR_ONLY.contains(&l.lighting.effect) {
        ui.label(RichText::new("This effect uses its own colours.").color(DIM));
        return;
    }
    ui.horizontal(|ui| {
        row_label(ui, "Single colour", "Off: the effect's rainbow palette");
        let mut single = l.lighting.color_mode == SINGLE_COLOUR;
        if toggle(ui, &mut single).changed() {
            l.lighting.color_mode = if single { SINGLE_COLOUR } else { MULTICOLOUR };
        }
        if single {
            for c in PALETTE {
                if swatch(ui, c, l.lighting.rgb == c) {
                    l.lighting.rgb = c;
                }
            }
            ui.color_edit_button_srgb(&mut l.lighting.rgb);
        }
    });
}

fn side_lights(ui: &mut Ui, l: &mut Layer) {
    let mut follow = l.side_lighting == l.lighting && l.colors[SIDE].iter().all(|c| c.mode == 0);
    ui.horizontal(|ui| {
        row_label(ui, "Match the keys", "Off: a colour of their own");
        if toggle(ui, &mut follow).changed() && !follow {
            l.side_lighting.effect = SINGLE_COLOR_EFFECT;
            l.side_lighting.color_mode = SINGLE_COLOUR;
            let side = l.side_lighting.rgb;
            l.colors[SIDE].fill(KeyLight { rgb: side, mode: 1 });
        }
    });
    if follow {
        l.side_lighting = l.lighting.clone();
        l.colors[SIDE].iter_mut().for_each(|c| c.mode = 0);
        return;
    }
    ui.horizontal(|ui| {
        row_label(ui, "Colour", "");
        let before = l.side_lighting.rgb;
        for c in PALETTE {
            if swatch(ui, c, l.side_lighting.rgb == c) {
                l.side_lighting.rgb = c;
            }
        }
        ui.color_edit_button_srgb(&mut l.side_lighting.rgb);
        if l.side_lighting.rgb != before {
            let side = l.side_lighting.rgb;
            l.colors[SIDE].fill(KeyLight { rgb: side, mode: 1 });
        }
    });
    ui.horizontal(|ui| {
        row_label(ui, "Brightness", "");
        ui.add(egui::Slider::new(&mut l.side_lighting.brightness, 0..=100).suffix("%"));
    });
}

/// Paint keys with `paint`; returns a newly picked paint colour.
fn per_key(ui: &mut Ui, l: &mut Layer, paint: [u8; 3]) -> Option<[u8; 3]> {
    let mut new_paint = None;
    ui.horizontal(|ui| {
        row_label(ui, "Per-key colours", "Painted keys override the effect");
        toggle(ui, &mut l.per_key_on);
    });
    ui.add_enabled_ui(l.per_key_on, |ui| {
        ui.horizontal(|ui| {
            row_label(ui, "Paint with", "Click keys to paint them; click again to clear");
            for c in PALETTE {
                if swatch(ui, c, paint == c) {
                    new_paint = Some(c);
                }
            }
            let mut custom = paint;
            if ui.color_edit_button_srgb(&mut custom).changed() {
                new_paint = Some(custom);
            }
        });
        ui.add_space(6.0);
        let colors = l.colors.clone();
        let clicked = keybed::show(ui, &|k| {
            let c = colors.get(k as usize).cloned().unwrap_or_default();
            Look { fill: (c.mode == 1).then(|| rgb(c.rgb)), ..Default::default() }
        });
        if let Some(k) = clicked {
            let slot = &mut l.colors[k as usize];
            *slot = if slot.mode == 1 && slot.rgb == paint {
                KeyLight { rgb: slot.rgb, mode: 0 }
            } else {
                KeyLight { rgb: paint, mode: 1 }
            };
        }
        ui.horizontal(|ui| {
            if ui.button(format!("{}  Paint every key", icon::PALETTE)).clicked() {
                for k in keybed::keys().chain([BADGE]) {
                    l.colors[k as usize] = KeyLight { rgb: paint, mode: 1 };
                }
            }
            if ui.button(format!("{}  Clear all", icon::X_CIRCLE)).clicked() {
                for k in keybed::keys().chain([BADGE]) {
                    l.colors[k as usize].mode = 0;
                }
            }
        });
    });
    new_paint
}
