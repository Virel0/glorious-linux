//! Firmware page: installed vs latest versions, the update flow and its log.
use super::format::product_display;
use super::App;
use crate::firmware::{self, Blocker, Part, Status};
use crate::gmmk3he::{self, Component};
use crate::icons as icon;
use crate::theme::*;
use eframe::egui::{self, RichText, Ui};

impl App {
    pub(super) fn firmware_tab(&mut self, ui: &mut Ui) {
        let mut check = false;
        page_header(ui, "Firmware", "Updates from Glorious for your mouse, receiver and keyboard", |ui| {
            let button = egui::Button::new(format!("{}  Check again", icon::ARROW_CLOCKWISE));
            check = ui.add_enabled(!self.fw_busy && !self.flashing, button).clicked();
            if self.fw_busy {
                ui.spinner();
            }
        });
        if let Some(e) = &self.fw_error {
            info_banner(ui, &format!("{} {e}", icon::WARNING));
            ui.add_space(14.0);
        }
        let mut flash = false;
        if let Some(st) = self.fw_status.clone() {
            versions_card(ui, &st);
            card(ui, icon::DOWNLOAD_SIMPLE, "Update", |ui| flash = self.update_controls(ui, &st));
            if self.flashing || self.fw_result.is_some() {
                self.flash_progress(ui);
            }
        } else if self.fw_busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Checking your devices and Glorious' latest release...").color(DIM));
            });
        }
        if let Some(components) = &self.keyboard.components {
            keyboard_card(ui, components);
        }
        if !self.fw_log.is_empty() {
            log_card(ui, &self.fw_log);
        }
        if check {
            self.check_firmware(ui.ctx());
        }
        if flash {
            self.flash(ui.ctx());
        }
    }

    /// Options, connection state, confirmations and the start button; returns true to start.
    fn update_controls(&mut self, ui: &mut Ui, st: &Status) -> bool {
        ui.horizontal(|ui| {
            row_label(ui, "What to update", "");
            let parts = [(Part::All, "Everything"), (Part::Mouse, "Mouse"), (Part::Receiver, "Receiver")];
            segmented(ui, &mut self.fw_part, &parts);
        });
        ui.horizontal(|ui| {
            row_label(ui, "Dry run", "Show every step without sending anything");
            toggle(ui, &mut self.fw_dry);
        });
        ui.horizontal(|ui| {
            row_label(ui, "Reinstall", "Flash even if already up to date");
            toggle(ui, &mut self.fw_force);
        });
        let (m, r) = st.plan(self.fw_part, self.fw_force);
        if m || r {
            self.connection_pills(ui, m, r);
        }
        let blocker = firmware::readiness((m, r), self.fw_dry, self.fw_cabled, self.fw_receiver, &self.fw_missing);
        if let Some(b) = blocker.as_ref().filter(|_| m || r) {
            info_banner(ui, &blocker_text(b));
        }
        let unverified = !st.bundle_verified() && (m || r) && !self.fw_dry;
        if unverified && blocker.is_none() {
            info_banner(ui, &format!(
                "{} This firmware is newer than this version of glorious-linux knows, so its checksum can't be checked. \
                 Updating glorious-linux is the safer option.",
                icon::WARNING
            ));
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.fw_allow_unverified, "");
                ui.label(RichText::new("Flash the unverified firmware anyway").color(TEXT));
            });
        }
        if !self.fw_dry && blocker.is_none() {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.fw_confirm, "");
                let text = "I'll keep the mouse and receiver plugged in until this finishes";
                ui.label(RichText::new(text).color(TEXT));
            });
        }
        ui.add_space(4.0);
        let ready = blocker.is_none() && (self.fw_dry || self.fw_confirm) && (!unverified || self.fw_allow_unverified);
        let mut start = false;
        ui.horizontal(|ui| {
            let text = if self.fw_dry { "Run dry run" } else { "Update firmware" };
            start = primary(ui, ready && !self.flashing && !self.mouse_busy, text);
            if !(m || r) {
                ui.label(RichText::new(Blocker::UpToDate.to_string()).color(DIM));
            }
        });
        start
    }

    /// Whether the devices the planned update needs are plugged in.
    fn connection_pills(&self, ui: &mut Ui, mouse: bool, receiver: bool) {
        ui.horizontal(|ui| {
            row_label(ui, "Connected", "");
            if mouse {
                let (glyph, text, color) = if self.fw_cabled {
                    (icon::CHECK_CIRCLE, "Mouse cable plugged in", OK)
                } else {
                    (icon::USB, "Mouse cable not plugged in", WARN)
                };
                pill(ui, glyph, text, color);
            }
            if receiver {
                let (glyph, text, color) = if self.fw_receiver {
                    (icon::CHECK_CIRCLE, "Receiver plugged in", OK)
                } else {
                    (icon::PLUG, "Receiver not plugged in", WARN)
                };
                pill(ui, glyph, text, color);
            }
        });
    }

    /// Step and progress of a running (or just finished) update.
    fn flash_progress(&self, ui: &mut Ui) {
        let title = match (self.flashing, self.fw_result) {
            (true, _) => "Updating - keep everything plugged in",
            (false, Some(true)) if self.fw_dry => "Dry run finished",
            (false, Some(true)) => "Update finished",
            _ => "Update stopped",
        };
        card(ui, icon::DOWNLOAD_SIMPLE, title, |ui| {
            match &self.fw_step {
                Some((n, of, step)) => self.step_progress(ui, *n, *of, step),
                None if self.flashing => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Downloading firmware from Glorious...").color(DIM));
                    });
                }
                None => {}
            }
            match self.fw_result {
                Some(true) if !self.fw_dry => {
                    let text =
                        format!("{} Done. Press \"Check again\" to confirm the new versions.", icon::CHECK_CIRCLE);
                    ui.label(RichText::new(text).color(OK));
                }
                Some(false) => {
                    let text = format!(
                        "{} Stopped - see the log. A device left in its bootloader is safe; run the update again.",
                        icon::WARNING
                    );
                    ui.label(RichText::new(text).color(ERR));
                }
                _ => {}
            }
        });
    }

    /// Step `n` of `of`: its own progress bar, then a thin one for the whole update.
    fn step_progress(&self, ui: &mut Ui, n: usize, of: usize, step: &str) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Step {n} of {of}")).color(DIM));
            ui.label(RichText::new(step).color(TEXT));
            if self.flashing {
                ui.spinner();
            }
        });
        let p = if self.fw_result == Some(true) && n == of { 1.0 } else { self.fw_progress };
        ui.add(egui::ProgressBar::new(p).text(format!("{:.0}%", p * 100.0)).desired_width(ui.available_width()));
        let overall = ((n - 1) as f32 + p) / of as f32;
        ui.add(egui::ProgressBar::new(overall).desired_height(6.0).desired_width(ui.available_width()));
    }
}

fn blocker_text(b: &Blocker) -> String {
    let glyph = match b {
        Blocker::UpToDate => return b.to_string(),
        Blocker::NoCable => icon::USB,
        Blocker::NoReceiver => icon::PLUG,
        Blocker::MissingTools(_) => icon::WARNING,
    };
    format!("{glyph} {b}")
}

/// One "installed / latest" line with an up-to-date pill.
fn version_row(ui: &mut Ui, name: &str, name_width: f32, have: &str, latest: &str, outdated: bool) {
    egui::Frame::new().fill(SURFACE2).corner_radius(10).inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.add_sized([name_width, 24.0], egui::Label::new(RichText::new(name).color(TEXT)));
            ui.label(RichText::new(format!("installed {have}")).color(DIM));
            ui.label(RichText::new(format!("latest {latest}")).color(FAINT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| status_pill(ui, outdated));
        });
    });
}

fn versions_card(ui: &mut Ui, st: &Status) {
    card(ui, icon::CPU, &format!("{}  -  {}", product_display(st.product), st.color), |ui| {
        version_row(ui, "Mouse", 120.0, &st.mouse, &st.latest_mouse, st.mouse_outdated());
        if let Some(r) = &st.receiver {
            let (have, latest) = (r.join(" / "), st.latest_receiver.join(" / "));
            version_row(ui, "Receiver", 120.0, &have, &latest, st.receiver_outdated());
        }
    });
}

fn keyboard_card(ui: &mut Ui, components: &[Component]) {
    card(ui, icon::KEYBOARD, gmmk3he::NAME, |ui| {
        for c in components {
            let name = match c.name {
                "device" => "Main firmware",
                "rf" => "Wireless chip",
                _ => "Lighting chip",
            };
            let installed = format!("{:04x}", c.installed);
            version_row(ui, name, 140.0, &installed, &c.latest_display(), c.outdated());
        }
        if components.iter().any(|c| c.outdated()) {
            ui.label(RichText::new("Keyboard updates still need Glorious CORE on Windows.").color(DIM));
        }
    });
}

fn log_card(ui: &mut Ui, log: &[String]) {
    card(ui, icon::WAVEFORM, "Log", |ui| {
        egui::Frame::new().fill(PANEL).corner_radius(10).inner_margin(12).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical().max_height(280.0).stick_to_bottom(true).show(ui, |ui| {
                for line in log {
                    let color = if line.starts_with("ERROR") {
                        ERR
                    } else if line.starts_with('[') {
                        ACCENT
                    } else {
                        DIM
                    };
                    ui.label(RichText::new(line).monospace().color(color));
                }
            });
        });
    });
}
