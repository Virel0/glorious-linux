//! Mouse lighting: effect, brightness, speed and colour, with a live preview.
use super::format::{mouse_state, pretty};
use super::App;
use crate::icons as icon;
use crate::settings::LightingSettings;
use crate::theme::*;
use crate::v3::EFFECTS;
use eframe::egui::{self, Color32, RichText, Sense, Stroke, Ui};
use std::time::Duration;

#[rustfmt::skip]
const PRESET_COLORS: [[u8; 3]; 8] = [
    [0xF5, 0xA6, 0x23], [0xFF, 0x3B, 0x30], [0xFF, 0x2D, 0x95], [0xAF, 0x52, 0xDE],
    [0x0A, 0x84, 0xFF], [0x00, 0xC7, 0xBE], [0x34, 0xC7, 0x59], [0xFF, 0xFF, 0xFF],
];

impl App {
    pub(super) fn lighting_tab(&mut self, ui: &mut Ui) {
        if !self.settings_loaded {
            self.mouse_problem(ui, &mouse_state(&self.mouse_error));
            return;
        }
        page_header(ui, "Lighting", "Effect, brightness and colour", |_| {});
        let l = &mut self.draft.lighting;
        egui::Frame::new().fill(SURFACE).stroke(Stroke::new(1.0_f32, BORDER)).corner_radius(14).inner_margin(12).show(
            ui,
            |ui| {
                ui.set_width(ui.available_width());
                lighting_preview(ui, l);
            },
        );
        ui.add_space(14.0);
        card(ui, icon::SPARKLE, "Effect", |ui| effect_grid(ui, l));
        card(ui, icon::SLIDERS, "Brightness and speed", |ui| {
            ui.horizontal(|ui| {
                row_label(ui, "Brightness on cable", "");
                ui.add(egui::Slider::new(&mut l.brightness, 0..=100).suffix("%"));
            });
            ui.horizontal(|ui| {
                row_label(ui, "Brightness on wireless", "Lower saves battery");
                ui.add(egui::Slider::new(&mut l.wireless_brightness, 0..=100).suffix("%"));
            });
            ui.horizontal(|ui| {
                row_label(ui, "Speed", "");
                ui.add(egui::Slider::new(&mut l.speed, 10..=100));
            });
        });
        card(ui, icon::PALETTE, "Colour", |ui| {
            ui.horizontal(|ui| {
                row_label(ui, "Single colour", "Off: the effect's own multicolour palette");
                toggle(ui, &mut l.custom_color);
            });
            ui.add_enabled_ui(l.custom_color, |ui| {
                ui.horizontal(|ui| {
                    row_label(ui, "Colour", "");
                    for c in PRESET_COLORS {
                        if swatch(ui, c, l.color == c) {
                            l.color = c;
                        }
                    }
                    ui.add_space(6.0);
                    ui.color_edit_button_srgb(&mut l.color).on_hover_text("Custom colour");
                });
            });
        });
    }
}

fn effect_grid(ui: &mut Ui, l: &mut LightingSettings) {
    egui::Grid::new("effects").num_columns(4).spacing([8.0, 8.0]).show(ui, |ui| {
        for (k, (name, _)) in EFFECTS.iter().enumerate() {
            let selected = l.effect == *name;
            let text = RichText::new(pretty(name)).color(if selected { ON_ACCENT } else { TEXT });
            let button = egui::Button::new(text)
                .fill(if selected { ACCENT } else { SURFACE2 })
                .min_size(egui::vec2(150.0, 36.0));
            if ui.add(button).clicked() {
                l.effect = name.to_string();
            }
            if k % 4 == 3 {
                ui.end_row();
            }
        }
    });
}

/// Animated approximation of the selected effect.
fn lighting_preview(ui: &mut Ui, l: &LightingSettings) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 64.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 12.0, PANEL);
    if l.effect == "off" {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Lighting off",
            egui::FontId::proportional(14.0),
            FAINT,
        );
        return;
    }
    let t = ui.input(|i| i.time) as f32;
    let speed = l.speed.max(10) as f32 / 50.0;
    let level = 0.25 + 0.75 * l.brightness as f32 / 100.0;
    let n = 64;
    let w = rect.width() / n as f32;
    for k in 0..n {
        let x = k as f32 / n as f32;
        let (hue, v) = match l.effect.as_str() {
            "solid" => (x, 1.0),
            "breathing" | "breathing-single" | "seamless-breathing" => {
                (t * 0.05, 0.55 + 0.45 * (t * speed * 2.0).sin())
            }
            "ripple" => (x * 0.6, 0.5 + 0.5 * ((x - 0.5).abs() * 18.0 - t * speed * 4.0).sin()),
            "fairies" | "ghost" => (x + t * 0.05, ((k as f32 * 7.31 + t * speed * 2.5).sin()).max(0.0).powi(3)),
            _ => (x + t * 0.15 * speed, 1.0),
        };
        let v = (v * level).clamp(0.0, 1.0);
        let color = if l.custom_color {
            let [r, g, b] = l.color;
            Color32::from_rgb((r as f32 * v) as u8, (g as f32 * v) as u8, (b as f32 * v) as u8)
        } else {
            egui::ecolor::Hsva::new(hue.rem_euclid(1.0), 0.85, v, 1.0).into()
        };
        let strip = egui::Rect::from_min_size(
            egui::pos2(rect.left() + k as f32 * w, rect.top()),
            egui::vec2(w + 1.0, rect.height()),
        );
        painter.rect_filled(strip, 0.0, color);
    }
    ui.ctx().request_repaint_after(Duration::from_millis(33));
}
