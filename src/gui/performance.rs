//! Mouse performance: DPI stages, polling rate, sensor and power settings.
use super::format::mouse_state;
use super::App;
use crate::icons as icon;
use crate::settings::{DpiStage, Performance};
use crate::theme::*;
use eframe::egui::{self, RichText, Stroke, Ui};

#[rustfmt::skip]
const POLLING_RATES: [(u16, &str); 7] =
    [(125, "125"), (250, "250"), (500, "500"), (1000, "1K"), (2000, "2K"), (4000, "4K"), (8000, "8K")];
const LIFT_OFF: [(u8, &str); 3] = [(0, "0.7 mm"), (1, "1.0 mm"), (2, "2.0 mm")];

impl App {
    pub(super) fn performance_tab(&mut self, ui: &mut Ui) {
        if !self.settings_loaded {
            self.mouse_problem(ui, &mouse_state(&self.mouse_error));
            return;
        }
        let mut reset = false;
        page_header(ui, "Performance", "Sensitivity, polling and power", |ui| {
            reset = ui.button(format!("{}  Glorious defaults", icon::ARROW_COUNTER_CLOCKWISE)).clicked();
        });
        if reset {
            self.draft.performance = Performance::default();
        }
        let p = &mut self.draft.performance;
        card(ui, icon::CROSSHAIR, "DPI stages", |ui| dpi_stages(ui, p));
        card(ui, icon::PULSE, "Polling rate", |ui| {
            ui.horizontal(|ui| {
                row_label(ui, "Wireless", "Higher rates use more battery");
                segmented(ui, &mut p.polling_wireless, &POLLING_RATES);
            });
            ui.horizontal(|ui| {
                row_label(ui, "USB cable", "");
                segmented(ui, &mut p.polling_wired, &POLLING_RATES);
            });
        });
        card(ui, icon::SLIDERS, "Sensor and buttons", |ui| sensor_and_buttons(ui, p));
        card(ui, icon::MOON, "Power saving", |ui| {
            ui.horizontal(|ui| {
                let hint = if p.sleep_minutes == 0 { "Never sleeps" } else { "Minutes without movement" };
                row_label(ui, "Sleep after", hint);
                ui.add(egui::Slider::new(&mut p.sleep_minutes, 0..=60).suffix(" min"));
            });
        });
    }
}

/// The six stage cards, wrapped to the available width.
fn dpi_stages(ui: &mut Ui, p: &mut Performance) {
    ui.label(
        RichText::new(
            "The DPI button cycles through the stages that are switched on. Click a stage's value to type it.",
        )
        .color(DIM),
    );
    ui.add_space(6.0);
    let per_row = ((ui.available_width() + 12.0) / (148.0 + 12.0)).floor().max(1.0) as usize;
    let active_now = p.active;
    let mut make_active = None;
    for (row, chunk) in p.stages.chunks_mut(per_row).enumerate() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(12.0, 12.0);
            for (j, s) in chunk.iter_mut().enumerate() {
                let i = row * per_row + j;
                if stage_card(ui, s, i, active_now == i) {
                    make_active = Some(i);
                }
            }
        });
    }
    if let Some(i) = make_active {
        p.active = i;
    }
}

/// One DPI stage; returns true when "Use" was clicked.
fn stage_card(ui: &mut Ui, s: &mut DpiStage, i: usize, active: bool) -> bool {
    let mut use_it = false;
    let stroke = if active { Stroke::new(2.0_f32, ACCENT) } else { Stroke::new(1.0_f32, BORDER) };
    egui::Frame::new().fill(SURFACE2).stroke(stroke).corner_radius(12).inner_margin(12).show(ui, |ui| {
        ui.set_width(124.0);
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.add_enabled_ui(s.enabled, |ui| ui.color_edit_button_srgb(&mut s.color));
                ui.label(RichText::new(format!("Stage {}", i + 1)).color(if s.enabled { TEXT } else { FAINT }));
            });
            ui.add_enabled(s.enabled, egui::DragValue::new(&mut s.dpi).range(50..=26000).speed(10.0).suffix(" DPI"));
            ui.horizontal(|ui| {
                toggle(ui, &mut s.enabled);
                if active {
                    ui.label(RichText::new("Active").size(12.5).color(ACCENT));
                } else if s.enabled && ui.small_button("Use").on_hover_text("Make this the active stage").clicked() {
                    use_it = true;
                }
            });
        });
    });
    use_it
}

fn sensor_and_buttons(ui: &mut Ui, p: &mut Performance) {
    ui.horizontal(|ui| {
        row_label(ui, "Lift-off distance", "How high the mouse can be lifted before tracking stops");
        segmented(ui, &mut p.lift_off, &LIFT_OFF);
    });
    ui.horizontal(|ui| {
        row_label(ui, "Motion sync", "Aligns sensor reads with polling");
        toggle(ui, &mut p.motion_sync);
    });
    ui.horizontal(|ui| {
        row_label(ui, "Mechanical debounce", "Side and DPI buttons");
        ui.add(egui::Slider::new(&mut p.debounce_mechanical, 0..=30).suffix(" ms"));
    });
    ui.horizontal(|ui| {
        row_label(ui, "Optical debounce", "Main buttons");
        ui.add(egui::Slider::new(&mut p.debounce_optical, 0..=30).suffix(" ms"));
    });
}
