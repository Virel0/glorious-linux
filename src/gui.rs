//! egui front end. All HID and network work runs on worker threads that report back over a
//! channel, so the window never blocks on the mouse's 30-200 ms report delays.
use crate::firmware::{self, Part};
use crate::gmmk3he;
use crate::settings::{self, LightingSettings, Performance, Settings};
use crate::v3::{key_names, Mouse, Snapshot, EFFECTS, FUNCTIONS};
use eframe::egui::{self, Color32, RichText};
use hidapi::HidApi;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

const ACCENT: Color32 = Color32::from_rgb(0xF5, 0xA6, 0x23);
const OK: Color32 = Color32::from_rgb(0x5C, 0xC8, 0x7A);
const WARN: Color32 = Color32::from_rgb(0xE8, 0x6A, 0x5C);
const POLLING_RATES: [u16; 7] = [125, 250, 500, 1000, 2000, 4000, 8000];
const LIFT_OFF: [(u8, &str); 3] = [(0, "0.7 mm"), (1, "1.0 mm"), (2, "2.0 mm")];

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Mouse,
    Performance,
    Lighting,
    Buttons,
    Firmware,
    Keyboard,
}

enum Applied {
    Performance(Performance),
    Lighting(LightingSettings),
    Buttons(Vec<(String, String)>),
}

enum Msg {
    Snapshot(Result<Snapshot, String>),
    Applied(Result<Applied, String>),
    FwStatus(Result<(firmware::Status, Vec<&'static str>), String>),
    FwLog(String),
    FwDone(Result<(), String>),
    Keyboard(Result<gmmk3he::Info, String>),
}

pub struct App {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    tab: Tab,
    mouse_busy: bool,
    snapshot: Option<Snapshot>,
    mouse_error: Option<String>,
    /// Last values known to be on the mouse, and the values being edited.
    saved: Settings,
    draft: Settings,
    have_saved_file: bool,
    toast: Option<(String, bool, Instant)>,
    fw_busy: bool,
    fw_status: Option<firmware::Status>,
    fw_missing: Vec<&'static str>,
    fw_error: Option<String>,
    fw_log: Vec<String>,
    fw_part: Part,
    fw_dry: bool,
    fw_force: bool,
    fw_confirm: bool,
    flashing: bool,
    kb_busy: bool,
    kb: Option<gmmk3he::Info>,
    kb_error: Option<String>,
    keys: Vec<(String, u8)>,
}

fn pretty(name: &str) -> String {
    let words: Vec<String> = name.split('-').map(|w| if w == "dpi" { "DPI".into() } else { w.to_string() }).collect();
    let s = words.join(" ");
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// "MOUSE_D3_WIRELESS" -> "Model D3 Wireless"
fn model_display(raw: &str) -> String {
    let words: Vec<String> = raw.split('_').skip(1).map(|w| {
        let w = w.to_lowercase();
        if w.len() == 2 { w.to_uppercase() } else { pretty(&w) }
    }).collect();
    if words.is_empty() { raw.to_string() } else { format!("Model {}", words.join(" ")) }
}

fn card(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style()).inner_margin(14).corner_radius(10).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).strong().size(15.0));
        ui.add_space(8.0);
        add(ui);
    });
    ui.add_space(12.0);
}

/// Apply / Revert row; returns (apply, revert).
fn apply_row(ui: &mut egui::Ui, dirty: bool, busy: bool) -> (bool, bool) {
    let mut out = (false, false);
    ui.horizontal(|ui| {
        let apply = egui::Button::new(RichText::new("Apply to mouse").strong()).fill(if dirty { ACCENT.gamma_multiply(0.55) } else { ui.visuals().widgets.inactive.weak_bg_fill });
        out.0 = ui.add_enabled(dirty && !busy, apply).clicked();
        out.1 = ui.add_enabled(dirty && !busy, egui::Button::new("Revert")).clicked();
        if busy {
            ui.spinner();
        } else if dirty {
            ui.label(RichText::new("unsaved changes").color(ACCENT));
        }
    });
    out
}

fn keyboard_app() -> Option<PathBuf> {
    let beside = std::env::current_exe().ok()?.with_file_name("inglorious-core");
    if beside.is_file() {
        return Some(beside);
    }
    firmware::which("inglorious-core")
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> App {
        let ctx = &cc.egui_ctx;
        ctx.set_theme(egui::Theme::Dark);
        ctx.style_mut_of(egui::Theme::Dark, |s| {
            s.spacing.item_spacing = egui::vec2(10.0, 8.0);
            s.spacing.button_padding = egui::vec2(10.0, 5.0);
            s.visuals.selection.bg_fill = ACCENT.gamma_multiply(0.7);
            s.visuals.selection.stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);
            s.visuals.hyperlink_color = ACCENT;
        });
        let loaded = Settings::load();
        let (tx, rx) = channel();
        let saved = loaded.clone().unwrap_or_default();
        let mut app = App {
            tx,
            rx,
            tab: Tab::Mouse,
            mouse_busy: false,
            snapshot: None,
            mouse_error: None,
            draft: saved.clone(),
            saved,
            have_saved_file: loaded.is_some(),
            toast: None,
            fw_busy: false,
            fw_status: None,
            fw_missing: Vec::new(),
            fw_error: None,
            fw_log: Vec::new(),
            fw_part: Part::All,
            fw_dry: true,
            fw_force: false,
            fw_confirm: false,
            flashing: false,
            kb_busy: false,
            kb: None,
            kb_error: None,
            keys: key_names(),
        };
        app.refresh_mouse(ctx);
        app.refresh_keyboard(ctx);
        app
    }

    fn spawn(&self, ctx: &egui::Context, job: impl FnOnce(&Sender<Msg>, &egui::Context) + Send + 'static) {
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            job(&tx, &ctx);
            ctx.request_repaint();
        });
    }

    fn with_mouse<T: Send + 'static>(
        &mut self,
        ctx: &egui::Context,
        f: impl FnOnce(&Mouse) -> anyhow::Result<T> + Send + 'static,
        wrap: impl FnOnce(Result<T, String>) -> Msg + Send + 'static,
    ) {
        self.mouse_busy = true;
        self.spawn(ctx, move |tx, _| {
            let r = HidApi::new().map_err(anyhow::Error::from).and_then(|api| f(&Mouse::open(&api, false)?));
            let _ = tx.send(wrap(r.map_err(|e| format!("{e:#}"))));
        });
    }

    fn refresh_mouse(&mut self, ctx: &egui::Context) {
        self.with_mouse(ctx, |m| m.snapshot(), Msg::Snapshot);
    }

    fn refresh_keyboard(&mut self, ctx: &egui::Context) {
        self.kb_busy = true;
        self.spawn(ctx, |tx, _| {
            let _ = tx.send(Msg::Keyboard(gmmk3he::read().map_err(|e| format!("{e:#}"))));
        });
    }

    fn check_firmware(&mut self, ctx: &egui::Context) {
        self.fw_busy = true;
        self.fw_error = None;
        self.spawn(ctx, |tx, _| {
            let r = firmware::status().map(|s| {
                let missing = firmware::missing_tools(s.receiver.is_some());
                (s, missing)
            });
            let _ = tx.send(Msg::FwStatus(r.map_err(|e| format!("{e:#}"))));
        });
    }

    fn flash(&mut self, ctx: &egui::Context) {
        let Some(st) = self.fw_status.clone() else { return };
        let (part, force, dry) = (self.fw_part, self.fw_force, self.fw_dry);
        self.flashing = true;
        self.fw_confirm = false;
        self.fw_log.clear();
        self.spawn(ctx, move |tx, ctx| {
            let log = |line: String| {
                let _ = tx.send(Msg::FwLog(line));
                ctx.request_repaint();
            };
            let r = firmware::flash(&st, part, force, dry, &log);
            let _ = tx.send(Msg::FwDone(r.map_err(|e| format!("{e:#}"))));
        });
    }

    fn toast(&mut self, text: impl Into<String>, ok: bool) {
        self.toast = Some((text.into(), ok, Instant::now()));
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Snapshot(r) => {
                self.mouse_busy = false;
                match r {
                    Ok(s) => {
                        self.mouse_error = None;
                        // Values the mouse can report override the saved copy.
                        let l = &s.lighting;
                        let device_lighting = LightingSettings {
                            effect: EFFECTS.iter().find(|e| e.1 == l.effect).map_or("rainbow", |e| e.0).to_string(),
                            brightness: l.wired_brightness,
                            wireless_brightness: l.wireless_brightness,
                            speed: l.speed,
                            ..self.saved.lighting.clone()
                        };
                        if self.draft.lighting == self.saved.lighting {
                            self.draft.lighting = device_lighting.clone();
                        }
                        self.saved.lighting = device_lighting;
                        if self.draft.performance.sleep_minutes == self.saved.performance.sleep_minutes {
                            self.draft.performance.sleep_minutes = s.sleep_minutes;
                        }
                        self.saved.performance.sleep_minutes = s.sleep_minutes;
                        self.snapshot = Some(s);
                    }
                    Err(e) => self.mouse_error = Some(e),
                }
            }
            Msg::Applied(r) => {
                self.mouse_busy = false;
                match r {
                    Ok(applied) => {
                        let what = match applied {
                            Applied::Performance(p) => {
                                self.saved.performance = p;
                                "Performance"
                            }
                            Applied::Lighting(l) => {
                                self.saved.lighting = l;
                                "Lighting"
                            }
                            Applied::Buttons(b) => {
                                self.saved.buttons = b;
                                "Button"
                            }
                        };
                        self.have_saved_file = true;
                        match self.saved.save() {
                            Ok(()) => self.toast(format!("{what} settings applied"), true),
                            Err(e) => self.toast(format!("{what} settings applied, but saving them failed: {e}"), false),
                        }
                    }
                    Err(e) => {
                        self.toast(format!("Could not apply: {e}"), false);
                        self.mouse_error = Some(e);
                    }
                }
            }
            Msg::FwStatus(r) => {
                self.fw_busy = false;
                match r {
                    Ok((s, missing)) => {
                        self.fw_status = Some(s);
                        self.fw_missing = missing;
                    }
                    Err(e) => self.fw_error = Some(e),
                }
            }
            Msg::FwLog(line) => self.fw_log.push(line),
            Msg::FwDone(r) => {
                self.flashing = false;
                match r {
                    Ok(()) => self.toast(if self.fw_dry { "Dry run finished" } else { "Firmware update finished" }, true),
                    Err(e) => {
                        self.fw_log.push(format!("ERROR: {e}"));
                        self.toast("Firmware update failed - see the log", false);
                    }
                }
            }
            Msg::Keyboard(r) => {
                self.kb_busy = false;
                match r {
                    Ok(i) => {
                        self.kb = Some(i);
                        self.kb_error = None;
                    }
                    Err(e) => self.kb_error = Some(e),
                }
            }
        }
    }

    fn nav(&mut self, ui: &mut egui::Ui) {
        ui.add_space(14.0);
        ui.label(RichText::new("Glorious").size(22.0).strong().color(ACCENT));
        ui.label(RichText::new("for Linux").weak());
        ui.add_space(18.0);
        for (tab, label) in [
            (Tab::Mouse, "Mouse"),
            (Tab::Performance, "Performance"),
            (Tab::Lighting, "Lighting"),
            (Tab::Buttons, "Buttons"),
            (Tab::Firmware, "Firmware"),
            (Tab::Keyboard, "Keyboard"),
        ] {
            let text = RichText::new(label).size(15.0);
            if ui.add_sized([ui.available_width(), 30.0], egui::Button::selectable(self.tab == tab, text)).clicked() {
                self.tab = tab;
            }
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.add_space(12.0);
            if let Some(s) = &self.snapshot {
                let b = &s.battery;
                ui.add(egui::ProgressBar::new(b.level as f32 / 100.0).text(format!("{}%", b.level)).desired_width(ui.available_width()));
                ui.label(RichText::new(model_display(&s.model)).strong());
            } else if self.mouse_busy {
                ui.spinner();
            }
        });
    }

    fn mouse_tab(&mut self, ui: &mut egui::Ui) {
        let mut refresh = false;
        ui.horizontal(|ui| {
            ui.heading("Mouse");
            if ui.add_enabled(!self.mouse_busy, egui::Button::new("Refresh")).clicked() {
                refresh = true;
            }
            if self.mouse_busy {
                ui.spinner();
            }
        });
        ui.add_space(8.0);
        if let Some(e) = &self.mouse_error {
            card(ui, "Not responding", |ui| {
                ui.label(RichText::new(e).color(WARN));
                ui.label("If the mouse is asleep, move it and press Refresh.");
            });
        }
        if let Some(s) = &self.snapshot {
            card(ui, "Device", |ui| {
                egui::Grid::new("dev").num_columns(2).spacing([24.0, 8.0]).show(ui, |ui| {
                    let row = |ui: &mut egui::Ui, k: &str, v: String| {
                        ui.label(RichText::new(k).weak());
                        ui.label(v);
                        ui.end_row();
                    };
                    row(ui, "Model", model_display(&s.model));
                    row(ui, "Colour", s.color.clone());
                    row(ui, "Connection", if s.via_receiver { "Wireless receiver".into() } else { "USB cable".into() });
                    row(ui, "Firmware", s.firmware.clone());
                    if let Some([a, b]) = &s.receiver_firmware {
                        row(ui, "Receiver firmware", format!("{a} / {b}"));
                    }
                    row(ui, "HID node", s.path.clone());
                });
            });
            card(ui, "Battery", |ui| {
                let b = &s.battery;
                ui.add(egui::ProgressBar::new(b.level as f32 / 100.0).text(format!("{}%  {}", b.level, b.status)).desired_width(320.0));
                ui.label(RichText::new(format!("{} mV  -  secondary cell {}% {}", b.millivolts, b.secondary_level, b.secondary_status)).weak());
            });
            card(ui, "Current lighting and power", |ui| {
                let l = &s.lighting;
                let effect = EFFECTS.iter().find(|e| e.1 == l.effect).map_or(l.effect.to_string(), |e| pretty(e.0));
                ui.label(format!("{effect}, brightness {} wired / {} wireless, speed {}", l.wired_brightness, l.wireless_brightness, l.speed));
                ui.label(if s.sleep_minutes == 0 { "Never sleeps".to_string() } else { format!("Sleeps after {} min idle", s.sleep_minutes) });
            });
        }
        if !self.have_saved_file {
            ui.label(RichText::new(
                "The mouse cannot report its DPI, polling, lift-off, debounce or button settings. Until you apply them here, \
                 the other tabs show Glorious' defaults, not what is on the mouse.",
            ).weak());
        }
        if refresh {
            self.refresh_mouse(ui.ctx());
        }
    }

    fn performance_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Performance");
        ui.add_space(8.0);
        let p = &mut self.draft.performance;
        card(ui, "DPI stages", |ui| {
            egui::Grid::new("dpi").num_columns(5).spacing([18.0, 8.0]).show(ui, |ui| {
                for h in ["Stage", "On", "DPI", "Colour", "Active"] {
                    ui.label(RichText::new(h).weak());
                }
                ui.end_row();
                for (i, s) in p.stages.iter_mut().enumerate() {
                    ui.label(format!("{}", i + 1));
                    ui.checkbox(&mut s.enabled, "");
                    ui.add_enabled(s.enabled, egui::DragValue::new(&mut s.dpi).range(50..=26000).speed(10.0).suffix(" DPI"));
                    ui.add_enabled_ui(s.enabled, |ui| ui.color_edit_button_srgb(&mut s.color));
                    ui.add_enabled(s.enabled, egui::RadioButton::new(p.active == i, "")).clicked().then(|| p.active = i);
                    ui.end_row();
                }
            });
            if !p.stages.get(p.active).is_some_and(|s| s.enabled) {
                ui.label(RichText::new("The active stage must be switched on.").color(WARN));
            }
        });
        card(ui, "Polling rate", |ui| {
            egui::Grid::new("poll").num_columns(2).spacing([18.0, 8.0]).show(ui, |ui| {
                for (label, value) in [("Wireless", &mut p.polling_wireless), ("USB cable", &mut p.polling_wired)] {
                    ui.label(label);
                    egui::ComboBox::from_id_salt(label).selected_text(format!("{value} Hz")).show_ui(ui, |ui| {
                        for r in POLLING_RATES {
                            ui.selectable_value(value, r, format!("{r} Hz"));
                        }
                    });
                    ui.end_row();
                }
            });
        });
        card(ui, "Sensor and buttons", |ui| {
            egui::Grid::new("sensor").num_columns(2).spacing([18.0, 10.0]).show(ui, |ui| {
                ui.label("Lift-off distance");
                ui.horizontal(|ui| {
                    for (v, label) in LIFT_OFF {
                        ui.selectable_value(&mut p.lift_off, v, label);
                    }
                });
                ui.end_row();
                ui.label("Motion sync");
                ui.checkbox(&mut p.motion_sync, "");
                ui.end_row();
                ui.label("Mechanical debounce");
                ui.add(egui::Slider::new(&mut p.debounce_mechanical, 0..=30).suffix(" ms"));
                ui.end_row();
                ui.label("Optical debounce");
                ui.add(egui::Slider::new(&mut p.debounce_optical, 0..=30).suffix(" ms"));
                ui.end_row();
                ui.label("Sleep after");
                ui.add(egui::Slider::new(&mut p.sleep_minutes, 0..=60).suffix(" min"));
                ui.end_row();
            });
            ui.label(RichText::new("Sleep 0 = never sleep.").weak());
        });
        let dirty = self.draft.performance != self.saved.performance;
        let (apply, revert) = apply_row(ui, dirty, self.mouse_busy || self.flashing);
        if ui.button("Reset to Glorious defaults").clicked() {
            self.draft.performance = Performance::default();
        }
        if revert {
            self.draft.performance = self.saved.performance.clone();
        }
        if apply {
            let (p, prev) = (self.draft.performance.clone(), self.saved.performance.clone());
            self.with_mouse(ui.ctx(), move |m| settings::apply_performance(m, &p, &prev).map(|_| p), |r| Msg::Applied(r.map(Applied::Performance)));
        }
    }

    fn lighting_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Lighting");
        ui.add_space(8.0);
        let l = &mut self.draft.lighting;
        card(ui, "Effect", |ui| {
            egui::Grid::new("light").num_columns(2).spacing([18.0, 10.0]).show(ui, |ui| {
                ui.label("Effect");
                egui::ComboBox::from_id_salt("effect").selected_text(pretty(&l.effect)).width(200.0).show_ui(ui, |ui| {
                    for (name, _) in EFFECTS {
                        ui.selectable_value(&mut l.effect, name.to_string(), pretty(name));
                    }
                });
                ui.end_row();
                ui.label("Brightness (USB)");
                ui.add(egui::Slider::new(&mut l.brightness, 0..=100).suffix("%"));
                ui.end_row();
                ui.label("Brightness (wireless)");
                ui.add(egui::Slider::new(&mut l.wireless_brightness, 0..=100).suffix("%"));
                ui.end_row();
                ui.label("Speed");
                ui.add(egui::Slider::new(&mut l.speed, 10..=100));
                ui.end_row();
                ui.label("Colour");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut l.custom_color, "Single colour");
                    ui.add_enabled_ui(l.custom_color, |ui| ui.color_edit_button_srgb(&mut l.color));
                });
                ui.end_row();
            });
            ui.label(RichText::new("Without \"Single colour\" the effect uses its multicolour palette.").weak());
        });
        let dirty = self.draft.lighting != self.saved.lighting;
        let (apply, revert) = apply_row(ui, dirty, self.mouse_busy || self.flashing);
        if revert {
            self.draft.lighting = self.saved.lighting.clone();
        }
        if apply {
            let l = self.draft.lighting.clone();
            self.with_mouse(ui.ctx(), move |m| settings::apply_lighting(m, &l).map(|_| l), |r| Msg::Applied(r.map(Applied::Lighting)));
        }
    }

    fn buttons_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Buttons");
        ui.add_space(8.0);
        let keys = &self.keys;
        let buttons = &mut self.draft.buttons;
        card(ui, "Assignments", |ui| {
            egui::Grid::new("buttons").num_columns(3).spacing([18.0, 10.0]).show(ui, |ui| {
                for (button, action) in buttons.iter_mut() {
                    ui.label(pretty(button));
                    let is_key = action.starts_with("key:");
                    let label = if is_key { "Keyboard key".to_string() } else { pretty(action) };
                    egui::ComboBox::from_id_salt(format!("act-{button}")).selected_text(label).width(170.0).show_ui(ui, |ui| {
                        for (name, ..) in FUNCTIONS {
                            ui.selectable_value(action, name.to_string(), pretty(name));
                        }
                        if ui.selectable_label(is_key, "Keyboard key").clicked() && !is_key {
                            *action = "key:a".into();
                        }
                    });
                    if let Some(key) = action.strip_prefix("key:").map(str::to_string) {
                        egui::ComboBox::from_id_salt(format!("key-{button}")).selected_text(key.to_uppercase()).width(110.0).show_ui(ui, |ui| {
                            for (name, _) in keys {
                                if ui.selectable_label(*name == key, name.to_uppercase()).clicked() {
                                    *action = format!("key:{name}");
                                }
                            }
                        });
                    } else {
                        ui.label("");
                    }
                    ui.end_row();
                }
            });
        });
        let dirty = self.draft.buttons != self.saved.buttons;
        let (apply, revert) = apply_row(ui, dirty, self.mouse_busy || self.flashing);
        if ui.button("Reset to Glorious defaults").clicked() {
            self.draft.buttons = Settings::default().buttons;
        }
        if revert {
            self.draft.buttons = self.saved.buttons.clone();
        }
        if apply {
            let b = self.draft.buttons.clone();
            self.with_mouse(ui.ctx(), move |m| settings::apply_buttons(m, &b).map(|_| b), |r| Msg::Applied(r.map(Applied::Buttons)));
        }
    }

    fn firmware_tab(&mut self, ui: &mut egui::Ui) {
        let mut check = false;
        ui.horizontal(|ui| {
            ui.heading("Firmware");
            check = ui.add_enabled(!self.fw_busy && !self.flashing, egui::Button::new("Check for updates")).clicked();
            if self.fw_busy {
                ui.spinner();
            }
        });
        ui.add_space(8.0);
        if let Some(e) = &self.fw_error {
            ui.label(RichText::new(e).color(WARN));
        }
        let mut flash = false;
        if let Some(st) = &self.fw_status {
            card(ui, &format!("{} ({})", st.product, st.color), |ui| {
                egui::Grid::new("fw").num_columns(3).spacing([24.0, 8.0]).show(ui, |ui| {
                    for h in ["", "Installed", "Latest"] {
                        ui.label(RichText::new(h).weak());
                    }
                    ui.end_row();
                    let row = |ui: &mut egui::Ui, name: &str, have: String, latest: String, outdated: bool| {
                        ui.label(name);
                        ui.label(have);
                        ui.label(if outdated { RichText::new(format!("{latest}  update available")).color(ACCENT) } else { RichText::new(format!("{latest}  up to date")).color(OK) });
                        ui.end_row();
                    };
                    row(ui, "Mouse", st.mouse.clone(), st.latest_mouse.clone(), st.mouse_outdated());
                    if let Some(r) = &st.receiver {
                        row(ui, "Receiver", r.join(" / "), st.latest_receiver.join(" / "), st.receiver_outdated());
                    }
                });
            });
            card(ui, "Update", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Part");
                    for (p, label) in [(Part::All, "Everything"), (Part::Mouse, "Mouse"), (Part::Receiver, "Receiver")] {
                        ui.selectable_value(&mut self.fw_part, p, label);
                    }
                });
                ui.checkbox(&mut self.fw_dry, "Dry run (show every step, send nothing)");
                ui.checkbox(&mut self.fw_force, "Flash even if already up to date");
                let (m, r) = st.plan(self.fw_part, self.fw_force);
                if m {
                    ui.label(RichText::new("The mouse update needs the USB cable plugged into the mouse.").color(ACCENT));
                }
                if !self.fw_dry && !self.fw_missing.is_empty() {
                    for tool in &self.fw_missing {
                        ui.label(RichText::new(format!("Missing: {tool}")).color(WARN));
                    }
                }
                if !self.fw_dry {
                    ui.checkbox(&mut self.fw_confirm, "I will keep the mouse and receiver plugged in until this finishes");
                }
                let ready = (m || r) && (self.fw_dry || (self.fw_confirm && self.fw_missing.is_empty()));
                let label = if self.fw_dry { "Run dry run" } else { "Flash firmware" };
                ui.horizontal(|ui| {
                    flash = ui.add_enabled(ready && !self.flashing && !self.mouse_busy, egui::Button::new(RichText::new(label).strong())).clicked();
                    if self.flashing {
                        ui.spinner();
                    }
                    if !(m || r) {
                        ui.label(RichText::new("Nothing to flash.").weak());
                    }
                });
            });
        } else if !self.fw_busy {
            ui.label("Press \"Check for updates\" to compare your mouse and receiver with Glorious' latest release.");
        }
        if !self.fw_log.is_empty() {
            card(ui, "Log", |ui| {
                egui::ScrollArea::vertical().max_height(260.0).stick_to_bottom(true).show(ui, |ui| {
                    for line in &self.fw_log {
                        ui.label(RichText::new(line).monospace().size(12.0));
                    }
                });
            });
        }
        if check {
            self.check_firmware(ui.ctx());
        }
        if flash {
            self.flash(ui.ctx());
        }
    }

    fn keyboard_tab(&mut self, ui: &mut egui::Ui) {
        let mut refresh = false;
        ui.horizontal(|ui| {
            ui.heading("Keyboard");
            refresh = ui.add_enabled(!self.kb_busy, egui::Button::new("Refresh")).clicked();
            if self.kb_busy {
                ui.spinner();
            }
        });
        ui.add_space(8.0);
        if let Some(e) = &self.kb_error {
            ui.label(RichText::new(e).color(WARN));
        }
        if let Some(kb) = &self.kb {
            card(ui, kb.name, |ui| {
                ui.label(format!("Active profile {}, layer {}", kb.profile + 1, kb.layer + 1));
                ui.add_space(6.0);
                egui::Grid::new("kbfw").num_columns(3).spacing([24.0, 8.0]).show(ui, |ui| {
                    for h in ["Firmware", "Installed", "Latest"] {
                        ui.label(RichText::new(h).weak());
                    }
                    ui.end_row();
                    for c in &kb.components {
                        ui.label(c.name.to_uppercase());
                        ui.label(format!("{:04x}", c.installed));
                        let latest = c.latest_display();
                        let text = if c.outdated() { RichText::new(format!("{latest}  update available")).color(ACCENT) } else { RichText::new(format!("{latest}  up to date")).color(OK) };
                        ui.label(text);
                        ui.end_row();
                    }
                });
                if kb.components.iter().any(|c| c.outdated()) {
                    ui.label(RichText::new("Keyboard updates still need Glorious CORE on Windows.").weak());
                }
            });
        }
        card(ui, "Lighting, actuation and keymap", |ui| {
            ui.label("Keyboard configuration is handled by inglorious-core-linux.");
            match keyboard_app() {
                Some(path) => {
                    if ui.button("Open keyboard configurator").clicked() {
                        match std::process::Command::new(&path).spawn() {
                            Ok(_) => self.toast("Keyboard configurator started", true),
                            Err(e) => self.toast(format!("Could not start {}: {e}", path.display()), false),
                        }
                    }
                }
                None => {
                    ui.label(RichText::new("inglorious-core was not found on your PATH.").color(WARN));
                    ui.hyperlink_to("Get it from github.com/tg5ee/inglorious-core-linux", "https://github.com/tg5ee/inglorious-core-linux");
                }
            }
        });
        if refresh {
            self.refresh_keyboard(ui.ctx());
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(msg) = self.rx.try_recv() {
            self.handle(msg);
        }
        egui::SidePanel::left("nav")
            .exact_width(190.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(ctx.style().visuals.extreme_bg_color).inner_margin(14))
            .show(ctx, |ui| self.nav(ui));
        if let Some((text, ok, at)) = &self.toast {
            if at.elapsed() < Duration::from_secs(5) {
                egui::TopBottomPanel::bottom("toast").show(ctx, |ui| {
                    ui.label(RichText::new(text).color(if *ok { OK } else { WARN }));
                });
                ctx.request_repaint_after(Duration::from_millis(500));
            }
        }
        if self.tab == Tab::Firmware && self.fw_status.is_none() && self.fw_error.is_none() && !self.fw_busy {
            self.check_firmware(ctx);
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.add_space(6.0);
                match self.tab {
                    Tab::Mouse => self.mouse_tab(ui),
                    Tab::Performance => self.performance_tab(ui),
                    Tab::Lighting => self.lighting_tab(ui),
                    Tab::Buttons => self.buttons_tab(ui),
                    Tab::Firmware => self.firmware_tab(ui),
                    Tab::Keyboard => self.keyboard_tab(ui),
                }
            });
        });
    }
}

pub fn run() -> anyhow::Result<()> {
    eframe::run_native(
        "Glorious",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_app_id("glorious-linux")
                .with_inner_size([960.0, 700.0])
                .with_min_inner_size([720.0, 520.0]),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
