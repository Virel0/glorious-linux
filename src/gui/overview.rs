//! Mouse overview: connection state, battery and a summary of the current settings.
use super::format::{battery_color, battery_icon, effect_name, model_display, mouse_state, pretty, MouseState};
use super::App;
use crate::icons as icon;
use crate::theme::*;
use crate::v3::{Battery, Snapshot};
use eframe::egui::{self, RichText, Stroke, Ui};

impl App {
    pub(super) fn overview_tab(&mut self, ui: &mut Ui) {
        let mut refresh = false;
        page_header(ui, "Overview", "Your mouse at a glance", |ui| {
            refresh = ui
                .add_enabled(!self.mouse_busy, egui::Button::new(format!("{}  Refresh", icon::ARROW_CLOCKWISE)))
                .clicked();
            if self.mouse_busy {
                ui.spinner();
            }
        });
        let state = mouse_state(&self.mouse_error);
        if self.snapshot.is_none() || state != MouseState::Ok {
            self.mouse_problem(ui, &state);
        }
        if let Some(s) = self.snapshot.clone() {
            self.hero(ui, &s, state == MouseState::Ok);
            ui.add_space(14.0);
            self.tiles(ui, &s);
        }
        if refresh {
            self.refresh_mouse(ui.ctx());
        }
    }

    fn tiles(&self, ui: &mut Ui, s: &Snapshot) {
        let p = &self.saved.performance;
        let active = p.stages.get(p.active).map_or(0, |s| s.dpi);
        let enabled = p.stages.iter().filter(|s| s.enabled).count();
        ui.columns(4, |cols| {
            tile(
                &mut cols[0],
                icon::CROSSHAIR,
                "DPI",
                &active.to_string(),
                &format!("stage {} of {enabled}", p.active + 1),
            );
            tile(&mut cols[1], icon::PULSE, "Polling", &format!("{} Hz", p.polling_wireless), "wireless");
            tile(
                &mut cols[2],
                icon::LIGHTBULB,
                "Lighting",
                &effect_name(s.lighting.effect),
                &format!("{}% brightness", s.lighting.wireless_brightness),
            );
            tile(
                &mut cols[3],
                icon::MOON,
                "Sleep",
                &if s.sleep_minutes == 0 { "Never".into() } else { format!("{} min", s.sleep_minutes) },
                "idle timeout",
            );
        });
    }

    /// Explains why the mouse isn't answering; shared by every mouse tab.
    pub(super) fn mouse_problem(&self, ui: &mut Ui, state: &MouseState) {
        if self.snapshot.is_some() && *state == MouseState::Asleep {
            info_banner(
                ui,
                &format!(
                    "{} Your mouse is asleep - showing the last values it reported. Move it to reconnect.",
                    icon::MOON
                ),
            );
            ui.add_space(14.0);
            return;
        }
        let (glyph, title, body) = match state {
            MouseState::Asleep => (
                icon::MOON,
                "Your mouse is asleep",
                "Move it to wake it up - this page reconnects on its own.".to_string(),
            ),
            MouseState::Missing => (
                icon::PLUG,
                "No mouse found",
                "Plug in the receiver or connect the mouse with its USB cable.".to_string(),
            ),
            MouseState::NoPermission => (
                icon::WARNING,
                "No permission to talk to the mouse",
                "Install the udev rule (the installer does this for you), then replug the receiver.".to_string(),
            ),
            MouseState::Other => (icon::WARNING, "Something went wrong", self.mouse_error.clone().unwrap_or_default()),
            MouseState::Ok => (icon::MOUSE, "Connecting...", "Looking for your mouse.".to_string()),
        };
        let glyph_color = if matches!(state, MouseState::Asleep | MouseState::Ok) { ACCENT } else { WARN };
        egui::Frame::new().fill(SURFACE).stroke(Stroke::new(1.0_f32, BORDER)).corner_radius(14).inner_margin(32).show(
            ui,
            |ui| {
                ui.set_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(glyph).size(44.0).color(glyph_color));
                    ui.add_space(6.0);
                    ui.label(RichText::new(title).family(heading_family()).size(20.0).color(TEXT));
                    ui.label(RichText::new(body).color(DIM));
                });
            },
        );
        ui.add_space(14.0);
    }

    fn hero(&self, ui: &mut Ui, s: &Snapshot, live: bool) {
        egui::Frame::new().fill(SURFACE).stroke(Stroke::new(1.0_f32, BORDER)).corner_radius(16).inner_margin(24).show(
            ui,
            |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| hero_details(ui, s, live));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        battery_gauge(ui, &s.battery)
                    });
                });
            },
        );
    }
}

/// Model name, connection pills and firmware versions.
fn hero_details(ui: &mut Ui, s: &Snapshot, live: bool) {
    ui.label(RichText::new(model_display(&s.model)).family(heading_family()).size(28.0).color(TEXT));
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        if live {
            pill(ui, icon::CHECK_CIRCLE, "Connected", OK);
        } else {
            pill(ui, icon::MOON, "Asleep", DIM);
        }
        let (glyph, link) = if s.via_receiver { (icon::WIFI_HIGH, "Wireless") } else { (icon::USB, "USB cable") };
        pill(ui, glyph, link, ACCENT);
        pill(ui, "", &s.color, DIM);
    });
    ui.add_space(14.0);
    egui::Grid::new("hero").num_columns(2).spacing([28.0, 6.0]).show(ui, |ui| {
        ui.label(RichText::new("Mouse firmware").color(FAINT));
        ui.label(RichText::new(&s.firmware).color(TEXT));
        ui.end_row();
        if let Some([a, b]) = &s.receiver_firmware {
            ui.label(RichText::new("Receiver firmware").color(FAINT));
            ui.label(RichText::new(format!("{a}  /  {b}")).color(TEXT));
            ui.end_row();
        }
    });
}

fn battery_gauge(ui: &mut Ui, b: &Battery) {
    ui.allocate_ui_with_layout(egui::vec2(150.0, 160.0), egui::Layout::top_down(egui::Align::Center), |ui| {
        ring(ui, b.level as f32 / 100.0, 118.0, &format!("{}%", b.level), battery_color(b.level));
        let glyph = battery_icon(b.level, b.status == "charging");
        ui.label(RichText::new(format!("{glyph} {}", pretty(b.status))).color(DIM));
    });
}
