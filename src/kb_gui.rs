//! Keyboard pages of the GUI (GMMK 3 HE 75% ANSI).
//!
//! Report building comes from inglorious-core-linux (vendored in `crate::magneton`), device I/O
//! from `crate::kbdevice`:
//! its worker thread owns the keyboard, its `State` holds every profile and layer, and its
//! `*_changes` functions turn an edit into exactly the reports Glorious CORE would send. The
//! keyboard can't report lighting, actuation or keymap back, so - like CORE - the last applied
//! values live in that library's state file (shared with the inglorious-core app).
use crate::gmmk3he::Component;
use crate::icons as icon;
use crate::kbdevice::{self as device, Command, Event};
use crate::keybed::{self, Look, BADGE};
use crate::magneton::protocol::{self as p, Actuation, KeyLight};
use crate::magneton::state::{self, State};
use crate::magneton::tables::{EFFECTS, KEYCODES, MULTICOLOUR, MULTICOLOUR_ONLY, SINGLE_COLOUR};
use crate::theme::*;
use eframe::egui::{self, Color32, RichText, Ui};
use std::collections::BTreeSet;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

pub const LAYERS: [&str; 3] = ["Base", "Fn", "Fn2"];
#[rustfmt::skip]
const RATES: [(u16, &str); 7] = [(125, "125"), (250, "250"), (500, "500"), (1000, "1K"), (2000, "2K"), (4000, "4K"), (8000, "8K")];
const SIDE: std::ops::Range<usize> = 128..158;
#[rustfmt::skip]
const PALETTE: [[u8; 3]; 8] = [
    [0xF5, 0xA6, 0x23], [0xFF, 0x3B, 0x30], [0xFF, 0x2D, 0x95], [0xAF, 0x52, 0xDE],
    [0x0A, 0x84, 0xFF], [0x00, 0xC7, 0xBE], [0x34, 0xC7, 0x59], [0xFF, 0xFF, 0xFF],
];
const INFO_REFRESH: Duration = Duration::from_secs(20);
/// Until the keyboard has answered once, keep trying more often.
const CONNECT_RETRY: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Home,
    Lighting,
    Actuation,
    Keymap,
}

pub struct Keyboard {
    tx: Sender<Command>,
    rx: Receiver<Event>,
    /// Last state known to be on the board, and the state being edited.
    saved: State,
    draft: State,
    had_state_file: bool,
    pub connected: bool,
    /// When the app started talking to the keyboard; the first read takes a few seconds.
    started: Instant,
    pub error: Option<String>,
    pub fw: Option<String>,
    pub components: Option<Vec<Component>>,
    /// Profile/layer the board is actually on.
    active: (u8, u8),
    pub busy: bool,
    progress: Option<(usize, usize)>,
    last_info: Instant,
    selected: BTreeSet<u8>,
    paint: [u8; 3],
    per_key_actuation: Actuation,
    live: bool,
    live_value: f32,
    /// Whether the keyboard's input interface (needed for live travel) could be opened.
    live_available: bool,
}

fn json<T: serde::Serialize>(v: &T) -> serde_json::Value {
    serde_json::to_value(v).unwrap_or_default()
}

/// "KeyA" -> "A", "ArrowLeft" -> "Arrow Left", "mouse_left" -> "Mouse Left".
pub fn key_display(name: &str) -> String {
    let base = name.strip_prefix("Key").or_else(|| name.strip_prefix("Digit")).unwrap_or(name);
    let mut out = String::new();
    for (i, c) in base.replace('_', " ").chars().enumerate() {
        if i > 0 && c.is_uppercase() && !out.ends_with(' ') {
            out.push(' ');
        }
        out.push(if i == 0 { c.to_ascii_uppercase() } else { c });
    }
    out
}

/// Bindable keycodes, one entry per code.
fn keycodes() -> Vec<(String, u8)> {
    let mut seen = BTreeSet::new();
    KEYCODES.iter().filter(|(_, code)| *code != 0 && seen.insert(*code)).map(|(n, c)| (key_display(n), *c)).collect()
}

fn code_name(code: u8) -> String {
    KEYCODES.iter().find(|(_, c)| *c == code).map_or(format!("0x{code:02x}"), |(n, _)| key_display(n))
}

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

impl Keyboard {
    pub fn new() -> Keyboard {
        let (tx, rx) = device::worker();
        let had_state_file = device::state_path().map(|p| p.exists()).unwrap_or(false);
        let (saved, error) = match device::load_state() {
            Ok(s) => (s, None),
            Err(e) => (State::default(), Some(format!("Couldn't read saved keyboard settings: {e:#}"))),
        };
        let kb = Keyboard {
            tx,
            rx,
            draft: saved.clone(),
            saved,
            had_state_file,
            connected: false,
            started: Instant::now(),
            error,
            fw: None,
            components: None,
            active: (0, 0),
            busy: false,
            progress: None,
            last_info: Instant::now(),
            selected: BTreeSet::new(),
            paint: PALETTE[0],
            per_key_actuation: Actuation::default(),
            live: false,
            live_value: 0.0,
            live_available: false,
        };
        let _ = kb.tx.send(Command::Info);
        kb
    }

    pub fn refresh(&mut self) {
        self.last_info = Instant::now();
        let _ = self.tx.send(Command::Info);
    }

    /// Handle worker events; returns toasts as (text, ok).
    pub fn poll(&mut self, page_open: bool) -> Vec<(String, bool)> {
        let mut toasts = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Info { fw, profile, layer, sleep, live_available } => {
                    self.connected = true;
                    self.live_available = live_available;
                    self.error = None;
                    self.active = (profile, layer);
                    // Follow the board's profile (it can be switched with Fn keys) without writing.
                    for s in [&mut self.saved, &mut self.draft] {
                        s.profile = profile;
                    }
                    let device_sleep = sleep.unwrap_or(0);
                    let pi = profile as usize;
                    if self.draft.profiles[pi].sleep == self.saved.profiles[pi].sleep {
                        self.draft.profiles[pi].sleep = device_sleep;
                    }
                    self.saved.profiles[pi].sleep = device_sleep;
                    if self.fw.as_deref() != Some(&fw) {
                        self.components = None;
                    }
                    self.fw = Some(fw);
                }
                Event::ActiveSlot { profile, layer } => self.active = (profile, layer),
                Event::Progress(i, n) => self.progress = Some((i, n)),
                Event::Saved(s) => {
                    let layer = self.draft.layer;
                    self.saved = *s;
                    self.saved.layer = layer;
                    self.had_state_file = true;
                    toasts.push(("Keyboard settings saved to the board".into(), true));
                }
                Event::Done => {
                    self.busy = false;
                    self.progress = None;
                }
                Event::Error(e) => {
                    self.busy = false;
                    self.progress = None;
                    if e.contains("not found") || e.contains("cannot open") {
                        self.connected = false;
                    } else {
                        toasts.push((format!("Keyboard: {e}"), false));
                    }
                    self.error = Some(e);
                }
                Event::Live(v) => self.live_value = v,
            }
        }
        let wait = if self.connected { INFO_REFRESH } else { CONNECT_RETRY };
        if !self.busy && self.last_info.elapsed() >= wait {
            self.refresh();
        }
        if self.live && !page_open {
            self.set_live(false);
        }
        toasts
    }

    pub fn shutdown(&self) {
        let _ = self.tx.send(Command::Shutdown);
    }

    fn set_live(&mut self, on: bool) {
        self.live = on;
        let _ = self.tx.send(Command::Live(on));
    }

    fn write(&mut self, mut reports: Vec<p::Report>, mut s: State) {
        if reports.is_empty() {
            return;
        }
        // Edits to another layer are indexed by layer, but the worker selects the edited slot
        // first; put the board back on the layer it was using afterwards.
        if s.layer != self.active.1 {
            if let Ok(back) = p::set_profile(s.profile, self.active.1) {
                reports.push(back);
            }
        }
        s.profiles[s.profile as usize].selected_layer = s.layer;
        self.busy = true;
        let _ = self.tx.send(Command::Write { reports, state: Some(Box::new(s)) });
        self.refresh();
    }

    // ------------------------------------------------------------------ save bar hooks

    pub fn dirty(&self, page: Page) -> bool {
        let (a, b) = (&self.draft, &self.saved);
        match page {
            Page::Home => state::performance_changes(a, b).is_ok_and(|r| !r.is_empty()),
            Page::Lighting => state::lighting_changes(a, b).is_ok_and(|r| !r.is_empty()),
            Page::Actuation => {
                let (x, y) = (a.current(), &b.profiles[a.profile as usize].layers[a.layer as usize]);
                json(&x.actuation) != json(&y.actuation) || json(&x.per_key) != json(&y.per_key)
            }
            Page::Keymap => state::key_changes(a, b).is_ok_and(|r| !r.is_empty()),
        }
    }

    pub fn apply(&mut self, page: Page) -> Result<(), String> {
        let (a, b) = (&self.draft, &self.saved);
        let reports = match page {
            Page::Home => state::performance_changes(a, b),
            // Lighting and performance belong to the profile; send them on the active layer.
            Page::Lighting => state::lighting_changes(a, b),
            Page::Actuation => state::actuation_reports(a),
            Page::Keymap => state::key_changes(a, b),
        }
        .map_err(|e| format!("{e:#}"))?;
        let mut s = self.draft.clone();
        if matches!(page, Page::Home | Page::Lighting) {
            s.layer = self.active.1;
        }
        self.write(reports, s);
        Ok(())
    }

    pub fn revert(&mut self, page: Page) {
        let pi = self.draft.profile as usize;
        let li = self.draft.layer as usize;
        let saved = self.saved.profiles[pi].clone();
        let d = &mut self.draft.profiles[pi];
        match page {
            Page::Home => (d.polling, d.debounce, d.sleep) = (saved.polling, saved.debounce, saved.sleep),
            Page::Lighting => d.layers[0] = saved.layers[0].clone(),
            Page::Actuation => {
                d.layers[li].actuation = saved.layers[li].actuation.clone();
                d.layers[li].per_key = saved.layers[li].per_key.clone();
            }
            Page::Keymap => {
                d.layers[li].bindings = saved.layers[li].bindings.clone();
                d.layers[li].advanced = saved.layers[li].advanced.clone();
            }
        }
    }

    fn select_layer(&mut self, layer: u8) {
        self.draft.layer = layer;
        self.saved.layer = layer;
        self.selected.clear();
    }

    fn switch_profile(&mut self, profile: u8) {
        if let Ok(r) = p::set_profile(profile, 0) {
            self.draft.profile = profile;
            self.draft.layer = 0;
            self.saved.profile = profile;
            self.saved.layer = 0;
            self.active = (profile, 0);
            let s = self.saved.clone();
            self.busy = true;
            let _ = self.tx.send(Command::Write { reports: vec![r], state: Some(Box::new(s)) });
            self.refresh();
        }
    }

    // ------------------------------------------------------------------ pages

    /// True during the first few seconds, before the keyboard has answered.
    pub fn connecting(&self) -> bool {
        !self.connected && self.error.is_none() && self.started.elapsed() < Duration::from_secs(10)
    }

    /// The keyboard is missing (as opposed to answering badly).
    fn missing(&self) -> bool {
        self.error.as_deref().is_some_and(|e| e.contains("not found") || e.contains("cannot open"))
            || (self.error.is_none() && !self.connecting())
    }

    /// While the keyboard isn't answering, keep the page (from the saved settings) and say why
    /// in a slim banner, instead of swapping the whole page out.
    fn status_banner(&self, ui: &mut Ui) {
        if self.connected {
            return;
        }
        let connecting = self.connecting();
        egui::Frame::new()
            .fill(if connecting { SURFACE } else { WARN.gamma_multiply(0.10) })
            .stroke(egui::Stroke::new(1.0_f32, if connecting { BORDER } else { WARN.gamma_multiply(0.35) }))
            .corner_radius(10)
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    if connecting {
                        ui.spinner();
                        ui.label(RichText::new("Connecting to your keyboard...").color(TEXT));
                    } else if self.missing() {
                        ui.label(RichText::new(icon::PLUG).size(16.0).color(WARN));
                        ui.label(
                            RichText::new(
                                "Keyboard not found - plug it in. Showing your saved settings; applying is paused.",
                            )
                            .color(TEXT),
                        );
                    } else {
                        ui.spinner();
                        let why = self.error.clone().unwrap_or_default();
                        ui.label(
                            RichText::new(format!("The keyboard didn't answer ({why}) - retrying...")).color(TEXT),
                        );
                    }
                });
            });
        ui.add_space(12.0);
    }

    fn layer_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            row_label(ui, "Layer", "Fn and Fn2 are active while their key is held");
            let mut layer = self.draft.layer;
            let options: Vec<(u8, &str)> = LAYERS.iter().enumerate().map(|(i, n)| (i as u8, *n)).collect();
            if segmented(ui, &mut layer, &options) {
                self.select_layer(layer);
            }
        });
    }

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
        if let Some(pr) = switch {
            self.switch_profile(pr);
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
        card(ui, icon::MOON, "Power saving", |ui| {
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
        });
        if !self.had_state_file {
            info_banner(ui, "The keyboard can't report its lighting, actuation or keymap. Until you apply them here, those pages show defaults - not what's on the board.");
        }
    }

    pub fn lighting(&mut self, ui: &mut Ui) {
        page_header(ui, "Key lighting", "Effects, side lights and per-key colours", |_| {});
        self.status_banner(ui);
        let paint = self.paint;
        let l = self.draft.lighting_state_mut();
        card(ui, icon::SPARKLE, "Effect", |ui| {
            egui::Grid::new("kb-effects").num_columns(4).spacing([8.0, 8.0]).show(ui, |ui| {
                for (k, (name, value)) in EFFECTS.iter().enumerate() {
                    let selected = l.lighting.effect == *value;
                    let text = RichText::new(*name).color(if selected { ON_ACCENT } else { TEXT });
                    let button = egui::Button::new(text).fill(if selected { ACCENT } else { SURFACE2 }).truncate();
                    if ui.add_sized([178.0, 36.0], button).clicked() {
                        l.lighting.effect = *value;
                        if MULTICOLOUR_ONLY.contains(value) {
                            l.lighting.color_mode = MULTICOLOUR;
                        } else if *value == 6 {
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
            } else {
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
        });
        card(ui, icon::LIGHTBULB, "Side lights", |ui| {
            let mut follow = l.side_lighting == l.lighting && l.colors[SIDE].iter().all(|c| c.mode == 0);
            ui.horizontal(|ui| {
                row_label(ui, "Match the keys", "Off: a colour of their own");
                if toggle(ui, &mut follow).changed() && !follow {
                    l.side_lighting.effect = 6;
                    l.side_lighting.color_mode = SINGLE_COLOUR;
                    let side = l.side_lighting.rgb;
                    l.colors[SIDE].fill(KeyLight { rgb: side, mode: 1 });
                }
            });
            if follow {
                l.side_lighting = l.lighting.clone();
                l.colors[SIDE].iter_mut().for_each(|c| c.mode = 0);
            } else {
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
        });
        let mut new_paint = None;
        card(ui, icon::PALETTE, "Per-key colours", |ui| {
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
        });
        if let Some(c) = new_paint {
            self.paint = c;
        }
    }

    pub fn actuation(&mut self, ui: &mut Ui) {
        page_header(ui, "Actuation", "Hall-effect press point, rapid trigger and dead zone", |_| {});
        self.status_banner(ui);
        self.layer_bar(ui);
        ui.add_space(8.0);
        let layer = self.draft.current_mut();
        card(ui, icon::GAUGE, "All keys", |ui| {
            let a = &mut layer.actuation;
            ui.horizontal(|ui| {
                row_label(ui, "Press point", "How far a key travels before it registers");
                ui.add(egui::Slider::new(&mut a.press, 0.1..=4.0).step_by(0.1).fixed_decimals(1).suffix(" mm"));
            });
            ui.horizontal(|ui| {
                row_label(ui, "Rapid trigger", "Re-trigger as soon as the key moves back up");
                toggle(ui, &mut a.rapid);
            });
            if a.rapid {
                ui.horizontal(|ui| {
                    row_label(ui, "Press sensitivity", "");
                    ui.add(
                        egui::Slider::new(&mut a.rapid_press, 0.1..=4.0).step_by(0.1).fixed_decimals(1).suffix(" mm"),
                    );
                });
                ui.horizontal(|ui| {
                    row_label(ui, "Release sensitivity", "");
                    ui.add(
                        egui::Slider::new(&mut a.rapid_release, 0.1..=4.0).step_by(0.1).fixed_decimals(1).suffix(" mm"),
                    );
                });
            }
            ui.horizontal(|ui| {
                row_label(ui, "Bottom dead zone", "Ignore travel this close to bottoming out");
                let mut dz = a.deadzone.unwrap_or(4.0);
                if ui.add(egui::Slider::new(&mut dz, 0.1..=4.0).step_by(0.1).fixed_decimals(1).suffix(" mm")).changed()
                {
                    a.deadzone = Some(dz);
                }
            });
        });
        let mut toggle_live = false;
        let live = self.live;
        let live_available = self.live_available;
        let live_value = self.live_value;
        let selected = self.selected.clone();
        let mut clicked = None;
        let mut set_selected = false;
        let mut clear_selected = false;
        let pk = &mut self.per_key_actuation;
        card(ui, icon::CROSSHAIR, "Per-key overrides", |ui| {
            ui.label(
                RichText::new("Select keys, then give them their own press point - e.g. a light touch for WASD.")
                    .color(DIM),
            );
            let per_key = layer.per_key.clone();
            clicked = keybed::show(ui, &|k| Look {
                value: per_key.get(&k).map(|a| format!("{:.1}", a.press)),
                selected: selected.contains(&k),
                highlight: per_key.contains_key(&k),
                ..Default::default()
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                row_label(ui, "Press point", &format!("{} key(s) selected", selected.len()));
                ui.add(egui::Slider::new(&mut pk.press, 0.1..=4.0).step_by(0.1).fixed_decimals(1).suffix(" mm"));
            });
            ui.horizontal(|ui| {
                row_label(ui, "Rapid trigger", "");
                toggle(ui, &mut pk.rapid);
                if pk.rapid {
                    ui.add(
                        egui::Slider::new(&mut pk.rapid_press, 0.1..=4.0).step_by(0.1).fixed_decimals(1).text("press"),
                    );
                    ui.add(
                        egui::Slider::new(&mut pk.rapid_release, 0.1..=4.0)
                            .step_by(0.1)
                            .fixed_decimals(1)
                            .text("release"),
                    );
                }
            });
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
                if toggle(ui, &mut on).changed() {
                    toggle_live = true;
                }
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

    pub fn keymap(&mut self, ui: &mut Ui) {
        page_header(ui, "Keymap", "Remap any key on any layer", |_| {});
        self.status_banner(ui);
        self.layer_bar(ui);
        ui.add_space(8.0);
        let selected = self.selected.iter().next().copied();
        let layer = self.draft.current_mut();
        let bindings = layer.bindings.clone();
        let advanced: BTreeSet<u8> = layer.advanced.keys().copied().collect();
        let mut clicked = None;
        card(ui, icon::KEYBOARD, "Keys", |ui| {
            ui.label(RichText::new("Click a key, then pick what it should send.").color(DIM));
            clicked = keybed::show(ui, &|k| Look {
                value: if advanced.contains(&k) {
                    Some("adv".into())
                } else {
                    bindings.get(&k).map(|codes| code_name(codes[0][2]))
                },
                selected: selected == Some(k),
                highlight: bindings.contains_key(&k) || advanced.contains(&k),
                ..Default::default()
            });
        });
        if let Some(k) = clicked {
            if k != BADGE {
                self.selected.clear();
                self.selected.insert(k);
            }
        }
        let Some(key) = self.selected.iter().next().copied() else { return };
        let layer = self.draft.current_mut();
        card(ui, icon::CURSOR_CLICK, &format!("{} key", keybed::label(key)), |ui| {
            if layer.advanced.contains_key(&key) {
                ui.label(RichText::new("This key has an advanced function (DKS, mod-tap, toggle or macro). Remapping it here replaces that.").color(WARN));
            }
            let current = layer.bindings.get(&key).map(|c| c[0][2]).or_else(|| state::default_code(key).ok());
            ui.horizontal(|ui| {
                row_label(ui, "Sends", "");
                let text = current.map_or("-".into(), code_name);
                egui::ComboBox::from_id_salt("kb-code").selected_text(text).width(220.0).height(360.0).show_ui(
                    ui,
                    |ui| {
                        for (name, code) in keycodes() {
                            if ui.selectable_label(current == Some(code), name).clicked() {
                                layer.advanced.remove(&key);
                                if state::default_code(key).ok() == Some(code) {
                                    layer.bindings.remove(&key);
                                } else {
                                    layer.bindings.insert(key, [[0, 0, code, 0], [0; 4], [0; 4], [0; 4]]);
                                }
                            }
                        }
                    },
                );
                if ui.button(format!("{}  Default", icon::ARROW_COUNTER_CLOCKWISE)).clicked() {
                    layer.bindings.remove(&key);
                    layer.advanced.remove(&key);
                }
            });
        });
    }

    pub fn progress(&self) -> Option<(usize, usize)> {
        self.progress
    }
}

impl Drop for Keyboard {
    fn drop(&mut self) {
        self.shutdown();
    }
}
