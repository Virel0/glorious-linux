//! Navigation and the connected-device chips at the bottom of the sidebar.
use super::format::{battery_color, battery_icon, model_display, mouse_state, MouseState};
use super::{App, Tab};
use crate::gmmk3he;
use crate::icons as icon;
use crate::kb_gui::Page;
use crate::theme::*;
use eframe::egui::{self, Color32, RichText, Ui};

/// Sidebar entry: (tab, icon, label).
type NavEntry = (Tab, &'static str, &'static str);

/// Sidebar sections: (heading, entries).
const NAV: [(&str, &[NavEntry]); 3] = [
    (
        "MOUSE",
        &[
            (Tab::Mouse, icon::MOUSE, "Overview"),
            (Tab::Performance, icon::GAUGE, "Performance"),
            (Tab::Lighting, icon::LIGHTBULB, "Lighting"),
            (Tab::Buttons, icon::CURSOR_CLICK, "Buttons"),
        ],
    ),
    (
        "KEYBOARD",
        &[
            (Tab::Kb(Page::Home), icon::KEYBOARD, "Keyboard"),
            (Tab::Kb(Page::Lighting), icon::PALETTE, "Key lighting"),
            (Tab::Kb(Page::Actuation), icon::SLIDERS, "Actuation"),
            (Tab::Kb(Page::Keymap), icon::CURSOR_CLICK, "Keymap"),
        ],
    ),
    ("UPDATES", &[(Tab::Firmware, icon::DOWNLOAD_SIMPLE, "Firmware")]),
];

impl App {
    pub(super) fn sidebar(&mut self, ui: &mut Ui) {
        ui.add_space(22.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new(icon::MOUSE).size(26.0).color(ACCENT));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(
                    RichText::new("Glorious").family(egui::FontFamily::Name("brand".into())).size(21.0).color(TEXT),
                );
                ui.label(RichText::new("for Linux").size(12.0).color(FAINT));
            });
        });
        ui.add_space(18.0);
        ui.spacing_mut().item_spacing.y = 4.0;
        for (heading, entries) in NAV {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                ui.label(RichText::new(heading).size(11.0).color(FAINT));
            });
            for &(tab, glyph, label) in entries {
                if nav_item(ui, glyph, label, self.tab == tab).clicked() {
                    self.tab = tab;
                }
            }
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.add_space(14.0);
            self.keyboard_chip(ui);
            self.mouse_chip(ui);
        });
    }

    fn keyboard_chip(&self, ui: &mut Ui) {
        if self.keyboard.connected {
            device_chip(ui, icon::KEYBOARD, gmmk3he::NAME, "Connected".into(), OK);
        } else if self.keyboard.connecting() {
            device_chip(ui, icon::KEYBOARD, "Keyboard", "Connecting...".into(), DIM);
        } else {
            device_chip(ui, icon::KEYBOARD, "Keyboard", "Not connected".into(), DIM);
        }
    }

    fn mouse_chip(&self, ui: &mut Ui) {
        match (&self.snapshot, mouse_state(&self.mouse_error)) {
            (Some(s), MouseState::Ok) => {
                let b = &s.battery;
                let status = format!("{} {}%", battery_icon(b.level, b.status == "charging"), b.level);
                device_chip(ui, icon::MOUSE, &model_display(&s.model), status, battery_color(b.level));
            }
            (Some(s), _) => {
                device_chip(ui, icon::MOUSE, &model_display(&s.model), format!("{} Asleep", icon::MOON), DIM)
            }
            (None, _) if self.mouse_busy => {
                ui.spinner();
            }
            (None, _) => device_chip(ui, icon::MOUSE, "Mouse", "Not connected".into(), DIM),
        }
    }
}

fn device_chip(ui: &mut Ui, glyph: &str, name: &str, status: String, color: Color32) {
    egui::Frame::new().fill(SURFACE).corner_radius(10).inner_margin(10).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(glyph).size(18.0).color(DIM));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new(name).size(13.0).color(TEXT));
                ui.label(RichText::new(status).size(12.0).color(color));
            });
        });
    });
}
