//! Actuation page: press point, rapid trigger, per-key overrides and live key travel.
use super::Keyboard;
use crate::icons as icon;
use crate::keybed::{self, Look, BADGE};
use crate::magneton::protocol::Actuation;
use crate::theme::*;
use eframe::egui::{self, RichText, Ui};
use std::time::Duration;

/// A 0.1-4.0 mm travel slider.
fn travel(value: &mut f32) -> egui::Slider<'_> {
    egui::Slider::new(value, 0.1..=4.0).step_by(0.1).fixed_decimals(1)
}

impl Keyboard {
    pub fn actuation(&mut self, ui: &mut Ui) {
        page_header(ui, "Actuation", "Hall-effect press point, rapid trigger and dead zone", |_| {});
        self.status_banner(ui);
        self.layer_bar(ui);
        ui.add_space(8.0);
        let a = &mut self.draft.current_mut().actuation;
        card(ui, icon::GAUGE, "All keys", |ui| all_keys(ui, a));
        self.per_key_overrides(ui);
        self.live_travel(ui);
    }

    fn per_key_overrides(&mut self, ui: &mut Ui) {
        let selected = self.selected.clone();
        let per_key = self.draft.current().per_key.clone();
        let pk = &mut self.per_key_actuation;
        let (mut clicked, mut set_selected, mut clear_selected) = (None, false, false);
        card(ui, icon::CROSSHAIR, "Per-key overrides", |ui| {
            ui.label(
                RichText::new("Select keys, then give them their own press point - e.g. a light touch for WASD.")
                    .color(DIM),
            );
            clicked = keybed::show(ui, &|k| Look {
                value: per_key.get(&k).map(|a| format!("{:.1}", a.press)),
                selected: selected.contains(&k),
                highlight: per_key.contains_key(&k),
                ..Default::default()
            });
            ui.add_space(6.0);
            override_controls(ui, pk, selected.len());
            ui.horizontal(|ui| {
                set_selected =
                    ui.add_enabled(!selected.is_empty(), egui::Button::new("Set for selected keys")).clicked();
                clear_selected = ui.add_enabled(!selected.is_empty(), egui::Button::new("Remove override")).clicked();
            });
        });
        if let Some(k) = clicked {
            if k != BADGE && !self.selected.remove(&k) {
                self.selected.insert(k);
            }
        }
        let layer = self.draft.current_mut();
        if set_selected {
            for k in &self.selected {
                let mut a = self.per_key_actuation.clone();
                a.deadzone = None;
                layer.per_key.insert(*k, a);
            }
        }
        if clear_selected {
            for k in &self.selected {
                layer.per_key.remove(k);
            }
        }
    }

    fn live_travel(&mut self, ui: &mut Ui) {
        let (live, live_value, live_available) = (self.live, self.live_value, self.live_available);
        let mut toggle_live = false;
        card(ui, icon::WAVEFORM, "Live key travel", |ui| {
            if !live_available {
                ui.label(
                    RichText::new(
                        "Off for security: live travel reads the keyboard's input interface, which also carries every \
                     keystroke, so the udev rule doesn't open it to apps. See the README to enable it.",
                    )
                    .color(DIM),
                );
                return;
            }
            ui.horizontal(|ui| {
                row_label(ui, "Show live travel", "Press any key to see how far it moves");
                let mut on = live;
                toggle_live = toggle(ui, &mut on).changed();
            });
            if live {
                ui.add(
                    egui::ProgressBar::new(live_value)
                        .text(format!("{:.1} mm", live_value * 4.0))
                        .desired_width(ui.available_width()),
                );
                ui.ctx().request_repaint_after(Duration::from_millis(33));
            }
        });
        if toggle_live {
            self.set_live(!live);
        }
    }
}

fn all_keys(ui: &mut Ui, a: &mut Actuation) {
    ui.horizontal(|ui| {
        row_label(ui, "Press point", "How far a key travels before it registers");
        ui.add(travel(&mut a.press).suffix(" mm"));
    });
    ui.horizontal(|ui| {
        row_label(ui, "Rapid trigger", "Re-trigger as soon as the key moves back up");
        toggle(ui, &mut a.rapid);
    });
    if a.rapid {
        ui.horizontal(|ui| {
            row_label(ui, "Press sensitivity", "");
            ui.add(travel(&mut a.rapid_press).suffix(" mm"));
        });
        ui.horizontal(|ui| {
            row_label(ui, "Release sensitivity", "");
            ui.add(travel(&mut a.rapid_release).suffix(" mm"));
        });
    }
    ui.horizontal(|ui| {
        row_label(ui, "Bottom dead zone", "Ignore travel this close to bottoming out");
        let mut dz = a.deadzone.unwrap_or(4.0);
        if ui.add(travel(&mut dz).suffix(" mm")).changed() {
            a.deadzone = Some(dz);
        }
    });
}

/// The press point and rapid trigger that "Set for selected keys" applies.
fn override_controls(ui: &mut Ui, pk: &mut Actuation, selected: usize) {
    ui.horizontal(|ui| {
        row_label(ui, "Press point", &format!("{selected} key(s) selected"));
        ui.add(travel(&mut pk.press).suffix(" mm"));
    });
    ui.horizontal(|ui| {
        row_label(ui, "Rapid trigger", "");
        toggle(ui, &mut pk.rapid);
        if pk.rapid {
            ui.add(travel(&mut pk.rapid_press).text("press"));
            ui.add(travel(&mut pk.rapid_release).text("release"));
        }
    });
}
