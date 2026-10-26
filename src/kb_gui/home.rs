//! Keyboard home page: connection, profile switching, polling, debounce and sleep.
use super::{Keyboard, LAYERS};
use crate::icons as icon;
use crate::magneton::state::Profile;
use crate::theme::*;
use eframe::egui::{self, RichText, Ui};

#[rustfmt::skip]
const RATES: [(u16, &str); 7] = [(125, "125"), (250, "250"), (500, "500"), (1000, "1K"), (2000, "2K"), (4000, "4K"), (8000, "8K")];

impl Keyboard {
    pub fn home(&mut self, ui: &mut Ui) {
        page_header(ui, "Keyboard", "GMMK 3 HE profiles, polling and power", |ui| {
            if ui.add_enabled(!self.busy, egui::Button::new(format!("{}  Refresh", icon::ARROW_CLOCKWISE))).clicked() {
                self.refresh();
            }
            if self.busy {
                ui.spinner();
            }
        });
        self.status_banner(ui);
        if let Some(profile) = self.board_card(ui) {
            self.switch_profile(profile);
        }
        ui.add_space(14.0);
        let pr = &mut self.draft.profiles[self.draft.profile as usize];
        card(ui, icon::PULSE, "Polling and debounce", |ui| {
            ui.horizontal(|ui| {
                row_label(ui, "Polling rate", "8K is lowest latency; 1K if you see stutter");
                segmented(ui, &mut pr.polling, &RATES);
            });
            ui.horizontal(|ui| {
                row_label(ui, "Debounce", "Hall-effect switches rarely need more than 5 ms");
                ui.add(egui::Slider::new(&mut pr.debounce, 0..=100).step_by(5.0).suffix(" ms"));
            });
        });
        card(ui, icon::MOON, "Power saving", |ui| power_saving(ui, pr));
        if !self.had_state_file {
            info_banner(ui, "The keyboard can't report its lighting, actuation or keymap. Until you apply them here, those pages show defaults - not what's on the board.");
        }
    }

    /// Name, connection pills and the profile picker; returns a profile to switch to.
    fn board_card(&self, ui: &mut Ui) -> Option<u8> {
        let mut switch = None;
        egui::Frame::new()
            .fill(SURFACE)
            .stroke(egui::Stroke::new(1.0_f32, BORDER))
            .corner_radius(16)
            .inner_margin(24)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(crate::gmmk3he::NAME).family(heading_family()).size(28.0).color(TEXT));
                ui.horizontal(|ui| {
                    if self.connected {
                        pill(ui, icon::CHECK_CIRCLE, "Connected", OK);
                        pill(ui, icon::USB, "Wired", ACCENT);
                        pill(ui, "", &format!("{} layer active", LAYERS[self.active.1 as usize % 3]), DIM);
                    } else {
                        pill(ui, icon::PLUG, "Not connected", DIM);
                    }
                });
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    row_label(ui, "Profile", "Switches the board immediately");
                    let mut profile = self.active.0;
                    if segmented(ui, &mut profile, &[(0, "Profile 1"), (1, "Profile 2"), (2, "Profile 3")])
                        && !self.busy
                    {
                        switch = Some(profile);
                    }
                });
            });
        switch
    }
}

fn power_saving(ui: &mut Ui, pr: &mut Profile) {
    ui.horizontal(|ui| {
        let mut on = pr.sleep != 0;
        row_label(ui, "Turn lighting off when idle", "");
        if toggle(ui, &mut on).changed() {
            pr.sleep = if on { 300 } else { 0 };
        }
    });
    if pr.sleep != 0 {
        ui.horizontal(|ui| {
            row_label(ui, "After", "");
            let mut minutes = (pr.sleep / 60).max(1);
            if ui.add(egui::Slider::new(&mut minutes, 1..=60).suffix(" min")).changed() {
                pr.sleep = minutes * 60;
            }
        });
    }
}
