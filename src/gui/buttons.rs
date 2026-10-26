//! Mouse button assignments.
use super::format::{mouse_state, pretty};
use super::App;
use crate::icons as icon;
use crate::settings::Settings;
use crate::theme::*;
use crate::v3::FUNCTIONS;
use eframe::egui::{self, RichText, Ui};

impl App {
    pub(super) fn buttons_tab(&mut self, ui: &mut Ui) {
        if !self.settings_loaded {
            self.mouse_problem(ui, &mouse_state(&self.mouse_error));
            return;
        }
        let mut reset = false;
        page_header(ui, "Buttons", "What each button does", |ui| {
            reset = ui.button(format!("{}  Glorious defaults", icon::ARROW_COUNTER_CLOCKWISE)).clicked();
        });
        if reset {
            self.draft.buttons = Settings::default().buttons;
        }
        let keys = &self.keys;
        let buttons = &mut self.draft.buttons;
        card(ui, icon::CURSOR_CLICK, "Assignments", |ui| {
            for (button, action) in buttons.iter_mut() {
                egui::Frame::new().fill(SURFACE2).corner_radius(10).inner_margin(egui::Margin::symmetric(14, 8)).show(
                    ui,
                    |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| button_row(ui, button, action, keys));
                    },
                );
                ui.add_space(2.0);
            }
        });
    }
}

/// Button name, its action, and the key it sends when the action is a keyboard key.
fn button_row(ui: &mut Ui, button: &str, action: &mut String, keys: &[(String, u8)]) {
    let name = if button == "dpi" { "DPI button".to_string() } else { format!("{} button", pretty(button)) };
    ui.add_sized([170.0, 30.0], egui::Label::new(RichText::new(name).color(TEXT)));
    let is_key = action.starts_with("key:");
    let label = if is_key { format!("{}  Keyboard key", icon::KEYBOARD) } else { pretty(action) };
    egui::ComboBox::from_id_salt(format!("act-{button}")).selected_text(label).width(190.0).show_ui(ui, |ui| {
        for (name, ..) in FUNCTIONS {
            ui.selectable_value(action, name.to_string(), pretty(name));
        }
        ui.separator();
        if ui.selectable_label(is_key, format!("{}  Keyboard key", icon::KEYBOARD)).clicked() && !is_key {
            *action = "key:a".into();
        }
    });
    let Some(key) = action.strip_prefix("key:").map(str::to_string) else { return };
    egui::ComboBox::from_id_salt(format!("key-{button}"))
        .selected_text(key.to_uppercase())
        .width(110.0)
        .height(320.0)
        .show_ui(ui, |ui| {
            for (name, _) in keys {
                if ui.selectable_label(*name == key, name.to_uppercase()).clicked() {
                    *action = format!("key:{name}");
                }
            }
        });
}
