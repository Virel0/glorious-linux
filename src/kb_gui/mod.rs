//! Keyboard pages of the GUI (GMMK 3 HE 75% ANSI).
//!
//! Report building comes from inglorious-core-linux (vendored in `crate::magneton`), device I/O
//! from `crate::kbdevice`:
//! its worker thread owns the keyboard, its `State` holds every profile and layer, and its
//! `*_changes` functions turn an edit into exactly the reports Glorious CORE would send. The
//! keyboard can't report lighting, actuation or keymap back, so - like CORE - the last applied
//! values live in that library's state file (shared with the inglorious-core app).
mod actuation;
mod home;
mod keymap;
mod lighting;

use crate::gmmk3he::Component;
use crate::icons as icon;
use crate::kbdevice::{self as device, Command, Event};
use crate::magneton::protocol::{self as p, Actuation};
use crate::magneton::state::{self, State};
use crate::theme::*;
use anyhow::Result;
use eframe::egui::{self, RichText, Ui};
use std::collections::BTreeSet;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

const LAYERS: [&str; 3] = ["Base", "Fn", "Fn2"];
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
            if let Some(toast) = self.on_event(event) {
                toasts.push(toast);
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

    fn on_event(&mut self, event: Event) -> Option<(String, bool)> {
        match event {
            Event::Info { fw, profile, layer, sleep, live_available } => {
                self.on_info(fw, profile, layer, sleep, live_available)
            }
            Event::ActiveSlot { profile, layer } => self.active = (profile, layer),
            Event::Progress(i, n) => self.progress = Some((i, n)),
            Event::Saved(s) => {
                let layer = self.draft.layer;
                self.saved = *s;
                self.saved.layer = layer;
                self.had_state_file = true;
                return Some(("Keyboard settings saved to the board".into(), true));
            }
            Event::Done => {
                self.busy = false;
                self.progress = None;
            }
            Event::Error(e) => {
                self.busy = false;
                self.progress = None;
                let toast = if e.contains("not found") || e.contains("cannot open") {
                    self.connected = false;
                    None
                } else {
                    Some((format!("Keyboard: {e}"), false))
                };
                self.error = Some(e);
                return toast;
            }
            Event::Live(v) => self.live_value = v,
        }
        None
    }

    fn on_info(&mut self, fw: String, profile: u8, layer: u8, sleep: Option<u16>, live_available: bool) {
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

    /// Send the page's edits to the board.
    ///
    /// # Errors
    /// When the edits can't be turned into reports; I/O errors arrive later as `Event::Error`.
    pub fn apply(&mut self, page: Page) -> Result<()> {
        let (a, b) = (&self.draft, &self.saved);
        let reports = match page {
            Page::Home => state::performance_changes(a, b),
            // Lighting and performance belong to the profile; send them on the active layer.
            Page::Lighting => state::lighting_changes(a, b),
            Page::Actuation => state::actuation_reports(a),
            Page::Keymap => state::key_changes(a, b),
        }?;
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

    pub fn progress(&self) -> Option<(usize, usize)> {
        self.progress
    }

    fn select_layer(&mut self, layer: u8) {
        self.draft.layer = layer;
        self.saved.layer = layer;
        self.selected.clear();
    }

    fn switch_profile(&mut self, profile: u8) {
        let Ok(r) = p::set_profile(profile, 0) else { return };
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

    // ------------------------------------------------------------------ shared page parts

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
}

impl Drop for Keyboard {
    fn drop(&mut self) {
        self.shutdown();
    }
}
