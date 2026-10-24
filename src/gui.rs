//! egui front end. All HID and network work runs on worker threads that report back over a
//! channel, so the window never blocks on the mouse's 30-200 ms report delays.
use crate::firmware::{self, Part};
use crate::gmmk3he;
use crate::settings::{self, LightingSettings, Performance, Settings};
use crate::theme::{self, *};
use crate::v3::{key_names, Mouse, Snapshot, EFFECTS, FUNCTIONS};
use eframe::egui::{self, Color32, RichText, Sense, Stroke, Ui};
use egui_phosphor::regular as icon;
use hidapi::HidApi;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

const POLLING_RATES: [(u16, &str); 7] =
    [(125, "125"), (250, "250"), (500, "500"), (1000, "1K"), (2000, "2K"), (4000, "4K"), (8000, "8K")];
const LIFT_OFF: [(u8, &str); 3] = [(0, "0.7 mm"), (1, "1.0 mm"), (2, "2.0 mm")];
const PRESET_COLORS: [[u8; 3]; 8] = [
    [0xF5, 0xA6, 0x23], [0xFF, 0x3B, 0x30], [0xFF, 0x2D, 0x95], [0xAF, 0x52, 0xDE],
    [0x0A, 0x84, 0xFF], [0x00, 0xC7, 0xBE], [0x34, 0xC7, 0x59], [0xFF, 0xFF, 0xFF],
];
const AUTO_REFRESH: Duration = Duration::from_secs(60);
const ASLEEP_RETRY: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Mouse,
    Performance,
    Lighting,
    Buttons,
    Firmware,
    Keyboard,
}

const TABS: [(Tab, &str, &str); 6] = [
    (Tab::Mouse, icon::MOUSE, "Overview"),
    (Tab::Performance, icon::GAUGE, "Performance"),
    (Tab::Lighting, icon::LIGHTBULB, "Lighting"),
    (Tab::Buttons, icon::CURSOR_CLICK, "Buttons"),
    (Tab::Firmware, icon::DOWNLOAD_SIMPLE, "Firmware"),
    (Tab::Keyboard, icon::KEYBOARD, "Keyboard"),
];

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

/// Why the mouse is not answering, from the error text.
enum MouseState {
    Ok,
    Asleep,
    Missing,
    NoPermission,
    Other,
}

pub struct App {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    tab: Tab,
    mouse_busy: bool,
    last_refresh: Instant,
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
    let words: Vec<String> = raw
        .split('_')
        .skip(1)
        .map(|w| if w.len() == 2 { w.to_string() } else { pretty(&w.to_lowercase()) })
        .collect();
    if words.is_empty() { raw.to_string() } else { format!("Model {}", words.join(" ")) }
}

/// "ModelD3Wireless" -> "Model D3 Wireless"
fn product_display(id: &str) -> String {
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for c in id.chars() {
        if let Some(p) = prev {
            if (c.is_uppercase() && !p.is_uppercase()) || (c.is_ascii_digit() && p.is_lowercase()) {
                out.push(' ');
            }
        }
        out.push(c);
        prev = Some(c);
    }
    out
}

fn effect_name(id: u8) -> String {
    EFFECTS.iter().find(|e| e.1 == id).map_or(format!("Effect {id}"), |e| pretty(e.0))
}

fn mouse_state(error: &Option<String>) -> MouseState {
    match error.as_deref() {
        None => MouseState::Ok,
        Some(e) if e.contains("asleep") => MouseState::Asleep,
        Some(e) if e.contains("no Glorious") => MouseState::Missing,
        Some(e) if e.contains("cannot open") => MouseState::NoPermission,
        Some(_) => MouseState::Other,
    }
}

fn battery_icon(level: u8, charging: bool) -> &'static str {
    match (charging, level) {
        (true, _) => icon::BATTERY_CHARGING,
        (_, 0..=15) => icon::BATTERY_EMPTY,
        (_, 16..=40) => icon::BATTERY_LOW,
        (_, 41..=70) => icon::BATTERY_MEDIUM,
        (_, 71..=90) => icon::BATTERY_HIGH,
        _ => icon::BATTERY_FULL,
    }
}

fn battery_color(level: u8) -> Color32 {
    match level {
        0..=15 => ERR,
        16..=30 => WARN,
        _ => OK,
    }
}

fn keyboard_app() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let candidates = [
        std::env::current_exe().ok().map(|p| p.with_file_name("inglorious-core")),
        Some(home.join(".local/bin/inglorious-core")),
        Some(home.join(".cargo/bin/inglorious-core")),
    ];
    candidates.into_iter().flatten().find(|p| p.is_file()).or_else(|| firmware::which("inglorious-core"))
}

/// Animated approximation of the selected effect.
fn lighting_preview(ui: &mut Ui, l: &LightingSettings) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 64.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 12.0, PANEL);
    if l.effect == "off" {
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Lighting off", egui::FontId::proportional(14.0), FAINT);
        return;
    }
    let t = ui.input(|i| i.time) as f32;
    let speed = l.speed.max(10) as f32 / 50.0;
    let level = 0.25 + 0.75 * l.brightness as f32 / 100.0;
    let n = 64;
    let w = rect.width() / n as f32;
    for k in 0..n {
        let x = k as f32 / n as f32;
        let (hue, v) = match l.effect.as_str() {
            "solid" => (x, 1.0),
            "breathing" | "breathing-single" | "seamless-breathing" => (t * 0.05, 0.55 + 0.45 * (t * speed * 2.0).sin()),
            "ripple" => (x * 0.6, 0.5 + 0.5 * ((x - 0.5).abs() * 18.0 - t * speed * 4.0).sin()),
            "fairies" | "ghost" => (x + t * 0.05, ((k as f32 * 7.31 + t * speed * 2.5).sin()).max(0.0).powi(3)),
            _ => (x + t * 0.15 * speed, 1.0),
        };
        let v = (v * level).clamp(0.0, 1.0);
        let color = if l.custom_color {
            let [r, g, b] = l.color;
            Color32::from_rgb((r as f32 * v) as u8, (g as f32 * v) as u8, (b as f32 * v) as u8)
        } else {
            egui::ecolor::Hsva::new(hue.rem_euclid(1.0), 0.85, v, 1.0).into()
        };
        let strip = egui::Rect::from_min_size(egui::pos2(rect.left() + k as f32 * w, rect.top()), egui::vec2(w + 1.0, rect.height()));
        painter.rect_filled(strip, 0.0, color);
    }
    ui.ctx().request_repaint_after(Duration::from_millis(33));
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> App {
        theme::install(&cc.egui_ctx);
        let loaded = Settings::load();
        let (tx, rx) = channel();
        let saved = loaded.clone().unwrap_or_default();
        let mut app = App {
            tx,
            rx,
            tab: Tab::Mouse,
            mouse_busy: false,
            last_refresh: Instant::now(),
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
        app.refresh_mouse(&cc.egui_ctx);
        app.refresh_keyboard(&cc.egui_ctx);
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
        self.last_refresh = Instant::now();
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

    /// Whether the current tab has unapplied edits.
    fn dirty(&self) -> bool {
        match self.tab {
            Tab::Performance => self.draft.performance != self.saved.performance,
            Tab::Lighting => self.draft.lighting != self.saved.lighting,
            Tab::Buttons => self.draft.buttons != self.saved.buttons,
            _ => false,
        }
    }

    fn apply(&mut self, ctx: &egui::Context) {
        match self.tab {
            Tab::Performance => {
                let (p, prev) = (self.draft.performance.clone(), self.saved.performance.clone());
                if !p.stages.get(p.active).is_some_and(|s| s.enabled) {
                    self.toast("The active DPI stage must be switched on", false);
                    return;
                }
                self.with_mouse(ctx, move |m| settings::apply_performance(m, &p, &prev).map(|_| p), |r| Msg::Applied(r.map(Applied::Performance)));
            }
            Tab::Lighting => {
                let l = self.draft.lighting.clone();
                self.with_mouse(ctx, move |m| settings::apply_lighting(m, &l).map(|_| l), |r| Msg::Applied(r.map(Applied::Lighting)));
            }
            Tab::Buttons => {
                let b = self.draft.buttons.clone();
                self.with_mouse(ctx, move |m| settings::apply_buttons(m, &b).map(|_| b), |r| Msg::Applied(r.map(Applied::Buttons)));
            }
            _ => {}
        }
    }

    fn revert(&mut self) {
        match self.tab {
            Tab::Performance => self.draft.performance = self.saved.performance.clone(),
            Tab::Lighting => self.draft.lighting = self.saved.lighting.clone(),
            Tab::Buttons => self.draft.buttons = self.saved.buttons.clone(),
            _ => {}
        }
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
                            Ok(()) => self.toast(format!("{what} settings saved to the mouse"), true),
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

    // ---------------------------------------------------------------------------------- sidebar

    fn sidebar(&mut self, ui: &mut Ui) {
        ui.add_space(22.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new(icon::MOUSE).size(26.0).color(ACCENT));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new("Glorious").family(egui::FontFamily::Name("brand".into())).size(21.0).color(TEXT));
                ui.label(RichText::new("for Linux").size(12.0).color(FAINT));
            });
        });
        ui.add_space(26.0);
        ui.spacing_mut().item_spacing.y = 4.0;
        for (tab, glyph, label) in TABS {
            if nav_item(ui, glyph, label, self.tab == tab).clicked() {
                self.tab = tab;
            }
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.add_space(14.0);
            if let Some(kb) = &self.kb {
                device_chip(ui, icon::KEYBOARD, kb.name, "Connected".into(), OK);
            }
            match (&self.snapshot, mouse_state(&self.mouse_error)) {
                (Some(s), MouseState::Ok) => {
                    let b = &s.battery;
                    let status = format!("{} {}%", battery_icon(b.level, b.status == "charging"), b.level);
                    device_chip(ui, icon::MOUSE, &model_display(&s.model), status, battery_color(b.level));
                }
                (Some(s), _) => device_chip(ui, icon::MOUSE, &model_display(&s.model), format!("{} Asleep", icon::MOON), DIM),
                (None, _) if self.mouse_busy => {
                    ui.spinner();
                }
                (None, _) => device_chip(ui, icon::MOUSE, "Mouse", "Not connected".into(), DIM),
            }
        });
    }

    // ---------------------------------------------------------------------------------- tabs

    fn overview_tab(&mut self, ui: &mut Ui) {
        let mut refresh = false;
        page_header(ui, "Overview", "Your mouse at a glance", |ui| {
            refresh = ui.add_enabled(!self.mouse_busy, egui::Button::new(format!("{}  Refresh", icon::ARROW_CLOCKWISE))).clicked();
            if self.mouse_busy {
                ui.spinner();
            }
        });
        let state = mouse_state(&self.mouse_error);
        if self.snapshot.is_none() || !matches!(state, MouseState::Ok) {
            self.mouse_problem(ui, &state);
        }
        if let Some(s) = self.snapshot.clone() {
            self.hero(ui, &s, matches!(state, MouseState::Ok));
            ui.add_space(14.0);
            let p = &self.saved.performance;
            let active = p.stages.get(p.active).map_or(0, |s| s.dpi);
            let enabled = p.stages.iter().filter(|s| s.enabled).count();
            let known = self.have_saved_file;
            ui.columns(4, |cols| {
                tile(&mut cols[0], icon::CROSSHAIR, "DPI", &if known { active.to_string() } else { "-".into() }, &if known { format!("stage {} of {enabled}", p.active + 1) } else { "not set here yet".into() });
                tile(&mut cols[1], icon::PULSE, "Polling", &if known { format!("{} Hz", p.polling_wireless) } else { "-".into() }, "wireless");
                tile(&mut cols[2], icon::LIGHTBULB, "Lighting", &effect_name(s.lighting.effect), &format!("{}% brightness", s.lighting.wireless_brightness));
                tile(&mut cols[3], icon::MOON, "Sleep", &if s.sleep_minutes == 0 { "Never".into() } else { format!("{} min", s.sleep_minutes) }, "idle timeout");
            });
            if !known {
                ui.add_space(14.0);
                info_banner(ui, "The mouse can't report its DPI, polling, debounce or button settings. Apply them once from the Performance and Buttons tabs and they'll show here.");
            }
        }
        if refresh {
            self.refresh_mouse(ui.ctx());
        }
    }

    fn mouse_problem(&self, ui: &mut Ui, state: &MouseState) {
        let (glyph, title, body) = match state {
            MouseState::Asleep => (icon::MOON, "Your mouse is asleep", "Move it to wake it up - this page reconnects on its own.".to_string()),
            MouseState::Missing => (icon::PLUG, "No mouse found", "Plug in the receiver or connect the mouse with its USB cable.".to_string()),
            MouseState::NoPermission => (icon::WARNING, "No permission to talk to the mouse", "Install the udev rule (the installer does this for you), then replug the receiver.".to_string()),
            MouseState::Other => (icon::WARNING, "Something went wrong", self.mouse_error.clone().unwrap_or_default()),
            MouseState::Ok => (icon::MOUSE, "Connecting...", "Looking for your mouse.".to_string()),
        };
        if self.snapshot.is_some() && matches!(state, MouseState::Asleep) {
            info_banner(ui, &format!("{} Your mouse is asleep - showing the last values it reported. Move it to reconnect.", icon::MOON));
            ui.add_space(14.0);
            return;
        }
        egui::Frame::new().fill(SURFACE).stroke(Stroke::new(1.0_f32, BORDER)).corner_radius(14).inner_margin(32).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(glyph).size(44.0).color(if matches!(state, MouseState::Asleep | MouseState::Ok) { ACCENT } else { WARN }));
                ui.add_space(6.0);
                ui.label(RichText::new(title).family(heading_family()).size(20.0).color(TEXT));
                ui.label(RichText::new(body).color(DIM));
            });
        });
        ui.add_space(14.0);
    }

    fn hero(&self, ui: &mut Ui, s: &Snapshot, live: bool) {
        egui::Frame::new().fill(SURFACE).stroke(Stroke::new(1.0_f32, BORDER)).corner_radius(16).inner_margin(24).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
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
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let b = &s.battery;
                    ui.allocate_ui_with_layout(egui::vec2(150.0, 160.0), egui::Layout::top_down(egui::Align::Center), |ui| {
                        ring(ui, b.level as f32 / 100.0, 118.0, &format!("{}%", b.level), battery_color(b.level));
                        ui.label(RichText::new(format!("{} {}", battery_icon(b.level, b.status == "charging"), pretty(b.status))).color(DIM));
                    });
                });
            });
        });
    }

    fn performance_tab(&mut self, ui: &mut Ui) {
        let mut reset = false;
        page_header(ui, "Performance", "Sensitivity, polling and power", |ui| {
            reset = ui.button(format!("{}  Glorious defaults", icon::ARROW_COUNTER_CLOCKWISE)).clicked();
        });
        if reset {
            self.draft.performance = Performance::default();
        }
        let p = &mut self.draft.performance;
        card(ui, icon::CROSSHAIR, "DPI stages", |ui| {
            ui.label(RichText::new("The DPI button cycles through the stages that are switched on. Click a stage's value to type it.").color(DIM));
            ui.add_space(6.0);
            let per_row = ((ui.available_width() + 12.0) / (148.0 + 12.0)).floor().max(1.0) as usize;
            let active_now = p.active;
            let mut make_active = None;
            for (row, chunk) in p.stages.chunks_mut(per_row).enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(12.0, 12.0);
                for (j, s) in chunk.iter_mut().enumerate() {
                    let i = row * per_row + j;
                    let active = active_now == i;
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
                                make_active = Some(i);
                            }
                        });
                        });
                    });
                }
            });
            }
            if let Some(i) = make_active {
                p.active = i;
            }
        });
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
        card(ui, icon::SLIDERS, "Sensor and buttons", |ui| {
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
        });
        card(ui, icon::MOON, "Power saving", |ui| {
            ui.horizontal(|ui| {
                row_label(ui, "Sleep after", if p.sleep_minutes == 0 { "Never sleeps" } else { "Minutes without movement" });
                ui.add(egui::Slider::new(&mut p.sleep_minutes, 0..=60).suffix(" min"));
            });
        });
    }

    fn lighting_tab(&mut self, ui: &mut Ui) {
        page_header(ui, "Lighting", "Effect, brightness and colour", |_| {});
        let l = &mut self.draft.lighting;
        egui::Frame::new().fill(SURFACE).stroke(Stroke::new(1.0_f32, BORDER)).corner_radius(14).inner_margin(12).show(ui, |ui| {
            ui.set_width(ui.available_width());
            lighting_preview(ui, l);
        });
        ui.add_space(14.0);
        card(ui, icon::SPARKLE, "Effect", |ui| {
            egui::Grid::new("effects").num_columns(4).spacing([8.0, 8.0]).show(ui, |ui| {
                for (k, (name, _)) in EFFECTS.iter().enumerate() {
                    let selected = l.effect == *name;
                    let text = RichText::new(pretty(name)).color(if selected { ON_ACCENT } else { TEXT });
                    let button = egui::Button::new(text).fill(if selected { ACCENT } else { SURFACE2 }).min_size(egui::vec2(150.0, 36.0));
                    if ui.add(button).clicked() {
                        l.effect = name.to_string();
                    }
                    if k % 4 == 3 {
                        ui.end_row();
                    }
                }
            });
        });
        card(ui, icon::SLIDERS, "Brightness and speed", |ui| {
            ui.horizontal(|ui| {
                row_label(ui, "Brightness on cable", "");
                ui.add(egui::Slider::new(&mut l.brightness, 0..=100).suffix("%"));
            });
            ui.horizontal(|ui| {
                row_label(ui, "Brightness on wireless", "Lower saves battery");
                ui.add(egui::Slider::new(&mut l.wireless_brightness, 0..=100).suffix("%"));
            });
            ui.horizontal(|ui| {
                row_label(ui, "Speed", "");
                ui.add(egui::Slider::new(&mut l.speed, 10..=100));
            });
        });
        card(ui, icon::PALETTE, "Colour", |ui| {
            ui.horizontal(|ui| {
                row_label(ui, "Single colour", "Off: the effect's own multicolour palette");
                toggle(ui, &mut l.custom_color);
            });
            ui.add_enabled_ui(l.custom_color, |ui| {
                ui.horizontal(|ui| {
                    row_label(ui, "Colour", "");
                    for c in PRESET_COLORS {
                        if swatch(ui, c, l.color == c) {
                            l.color = c;
                        }
                    }
                    ui.add_space(6.0);
                    ui.color_edit_button_srgb(&mut l.color).on_hover_text("Custom colour");
                });
            });
        });
    }

    fn buttons_tab(&mut self, ui: &mut Ui) {
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
                egui::Frame::new().fill(SURFACE2).corner_radius(10).inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
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
                        if let Some(key) = action.strip_prefix("key:").map(str::to_string) {
                            egui::ComboBox::from_id_salt(format!("key-{button}")).selected_text(key.to_uppercase()).width(110.0).height(320.0).show_ui(ui, |ui| {
                                for (name, _) in keys {
                                    if ui.selectable_label(*name == key, name.to_uppercase()).clicked() {
                                        *action = format!("key:{name}");
                                    }
                                }
                            });
                        }
                    });
                });
                ui.add_space(2.0);
            }
        });
    }

    fn firmware_tab(&mut self, ui: &mut Ui) {
        let mut check = false;
        page_header(ui, "Firmware", "Mouse and receiver updates from Glorious", |ui| {
            check = ui.add_enabled(!self.fw_busy && !self.flashing, egui::Button::new(format!("{}  Check again", icon::ARROW_CLOCKWISE))).clicked();
            if self.fw_busy {
                ui.spinner();
            }
        });
        if let Some(e) = &self.fw_error {
            info_banner(ui, &format!("{} {e}", icon::WARNING));
            ui.add_space(14.0);
        }
        let mut flash = false;
        if let Some(st) = &self.fw_status {
            card(ui, icon::CPU, &format!("{}  -  {}", product_display(st.product), st.color), |ui| {
                let row = |ui: &mut Ui, name: &str, have: String, latest: String, outdated: bool| {
                    egui::Frame::new().fill(SURFACE2).corner_radius(10).inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.add_sized([120.0, 24.0], egui::Label::new(RichText::new(name).color(TEXT)));
                            ui.label(RichText::new(format!("installed {have}")).color(DIM));
                            ui.label(RichText::new(format!("latest {latest}")).color(FAINT));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| status_pill(ui, outdated));
                        });
                    });
                };
                row(ui, "Mouse", st.mouse.clone(), st.latest_mouse.clone(), st.mouse_outdated());
                if let Some(r) = &st.receiver {
                    row(ui, "Receiver", r.join(" / "), st.latest_receiver.join(" / "), st.receiver_outdated());
                }
            });
            card(ui, icon::DOWNLOAD_SIMPLE, "Update", |ui| {
                ui.horizontal(|ui| {
                    row_label(ui, "What to update", "");
                    segmented(ui, &mut self.fw_part, &[(Part::All, "Everything"), (Part::Mouse, "Mouse"), (Part::Receiver, "Receiver")]);
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
                if m {
                    info_banner(ui, &format!("{} The mouse update needs the USB cable plugged into the mouse.", icon::USB));
                }
                if !self.fw_dry {
                    for tool in &self.fw_missing {
                        info_banner(ui, &format!("{} Missing {tool}", icon::WARNING));
                    }
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.fw_confirm, "");
                        ui.label(RichText::new("I'll keep the mouse and receiver plugged in until this finishes").color(TEXT));
                    });
                }
                ui.add_space(4.0);
                let ready = (m || r) && (self.fw_dry || (self.fw_confirm && self.fw_missing.is_empty()));
                ui.horizontal(|ui| {
                    flash = primary(ui, ready && !self.flashing && !self.mouse_busy, if self.fw_dry { "Run dry run" } else { "Update firmware" });
                    if self.flashing {
                        ui.spinner();
                    }
                    if !(m || r) {
                        ui.label(RichText::new("Everything is up to date - nothing to flash.").color(DIM));
                    }
                });
            });
        } else if self.fw_busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Checking your devices and Glorious' latest release...").color(DIM));
            });
        }
        if !self.fw_log.is_empty() {
            card(ui, icon::WAVEFORM, "Log", |ui| {
                egui::Frame::new().fill(PANEL).corner_radius(10).inner_margin(12).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::ScrollArea::vertical().max_height(280.0).stick_to_bottom(true).show(ui, |ui| {
                        for line in &self.fw_log {
                            let color = if line.starts_with("ERROR") { ERR } else if line.starts_with('[') { ACCENT } else { DIM };
                            ui.label(RichText::new(line).monospace().color(color));
                        }
                    });
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

    fn keyboard_tab(&mut self, ui: &mut Ui) {
        let mut refresh = false;
        page_header(ui, "Keyboard", "GMMK 3 HE status and configuration", |ui| {
            refresh = ui.add_enabled(!self.kb_busy, egui::Button::new(format!("{}  Refresh", icon::ARROW_CLOCKWISE))).clicked();
            if self.kb_busy {
                ui.spinner();
            }
        });
        if let Some(e) = &self.kb_error {
            info_banner(ui, &format!("{} {e}", icon::WARNING));
            ui.add_space(14.0);
        }
        if let Some(kb) = &self.kb {
            egui::Frame::new().fill(SURFACE).stroke(Stroke::new(1.0_f32, BORDER)).corner_radius(16).inner_margin(24).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(kb.name).family(heading_family()).size(28.0).color(TEXT));
                ui.horizontal(|ui| {
                    pill(ui, icon::CHECK_CIRCLE, "Connected", OK);
                    pill(ui, "", &format!("Profile {}  -  layer {}", kb.profile + 1, kb.layer + 1), ACCENT);
                });
                ui.add_space(10.0);
                for c in &kb.components {
                    egui::Frame::new().fill(SURFACE2).corner_radius(10).inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let name = match c.name { "device" => "Main firmware", "rf" => "Wireless chip", _ => "Lighting chip" };
                            ui.add_sized([140.0, 24.0], egui::Label::new(RichText::new(name).color(TEXT)));
                            ui.label(RichText::new(format!("installed {:04x}", c.installed)).color(DIM));
                            ui.label(RichText::new(format!("latest {}", c.latest_display())).color(FAINT));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| status_pill(ui, c.outdated()));
                        });
                    });
                    ui.add_space(2.0);
                }
                if kb.components.iter().any(|c| c.outdated()) {
                    ui.label(RichText::new("Keyboard updates still need Glorious CORE on Windows.").color(DIM));
                }
            });
            ui.add_space(14.0);
        }
        card(ui, icon::SLIDERS, "Lighting, actuation and keymap", |ui| {
            ui.label(RichText::new("Per-key RGB, actuation points, rapid trigger and the keymap are configured in inglorious-core-linux.").color(DIM));
            ui.add_space(4.0);
            match keyboard_app() {
                Some(path) => {
                    if primary(ui, true, &format!("{}  Open keyboard configurator", icon::ARROW_SQUARE_OUT)) {
                        match std::process::Command::new(&path).spawn() {
                            Ok(_) => self.toast("Keyboard configurator started", true),
                            Err(e) => self.toast(format!("Could not start {}: {e}", path.display()), false),
                        }
                    }
                }
                None => {
                    info_banner(ui, &format!("{} inglorious-core isn't installed. Re-run the installer and answer yes to the keyboard configurator.", icon::INFO));
                    ui.hyperlink_to("github.com/tg5ee/inglorious-core-linux", "https://github.com/tg5ee/inglorious-core-linux");
                }
            }
        });
        if refresh {
            self.refresh_keyboard(ui.ctx());
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

fn info_banner(ui: &mut Ui, text: &str) {
    egui::Frame::new()
        .fill(ACCENT.gamma_multiply(0.08))
        .stroke(Stroke::new(1.0_f32, ACCENT.gamma_multiply(0.3)))
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).color(TEXT));
        });
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(msg) = self.rx.try_recv() {
            self.handle(msg);
        }
        // Keep the battery fresh, and reconnect on its own once a sleeping mouse wakes up.
        let wait = if self.mouse_error.is_some() { ASLEEP_RETRY } else { AUTO_REFRESH };
        if !self.mouse_busy && !self.flashing && self.last_refresh.elapsed() >= wait {
            self.refresh_mouse(ctx);
        }
        ctx.request_repaint_after(Duration::from_secs(1));
        if self.tab == Tab::Firmware && self.fw_status.is_none() && self.fw_error.is_none() && !self.fw_busy {
            self.check_firmware(ctx);
        }
        if self.dirty() && !self.mouse_busy && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            self.apply(ctx);
        }

        egui::SidePanel::left("nav")
            .exact_width(220.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin { left: 12, right: 12, top: 0, bottom: 0 }))
            .show(ctx, |ui| self.sidebar(ui));

        if self.dirty() {
            egui::TopBottomPanel::bottom("savebar")
                .frame(egui::Frame::new().fill(SURFACE).stroke(Stroke::new(1.0_f32, BORDER)).inner_margin(egui::Margin::symmetric(24, 12)))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(icon::PENCIL_SIMPLE).size(16.0).color(ACCENT));
                        ui.label(RichText::new("Unsaved changes").color(TEXT));
                        ui.label(RichText::new("Ctrl+S").size(12.0).color(FAINT));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if primary(ui, !self.mouse_busy && !self.flashing, "Apply to mouse") {
                                self.apply(ctx);
                            }
                            if ui.add_enabled(!self.mouse_busy, egui::Button::new("Revert").min_size(egui::vec2(90.0, 34.0))).clicked() {
                                self.revert();
                            }
                            if self.mouse_busy {
                                ui.spinner();
                            }
                        });
                    });
                });
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin { left: 32, right: 32, top: 28, bottom: 0 }))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_max_width(ui.available_width().min(980.0));
                    match self.tab {
                        Tab::Mouse => self.overview_tab(ui),
                        Tab::Performance => self.performance_tab(ui),
                        Tab::Lighting => self.lighting_tab(ui),
                        Tab::Buttons => self.buttons_tab(ui),
                        Tab::Firmware => self.firmware_tab(ui),
                        Tab::Keyboard => self.keyboard_tab(ui),
                    }
                    ui.add_space(24.0);
                });
            });

        if let Some((text, ok, at)) = &self.toast {
            let age = at.elapsed().as_secs_f32();
            if age < 4.0 {
                let alpha = (4.0 - age).min(1.0);
                egui::Area::new(egui::Id::new("toast"))
                    .order(egui::Order::Foreground)
                    .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-24.0, if self.dirty() { -80.0 } else { -24.0 }))
                    .interactable(false)
                    .show(ctx, |ui| {
                        let color = if *ok { OK } else { ERR };
                        egui::Frame::new()
                            .fill(SURFACE2.gamma_multiply(alpha))
                            .stroke(Stroke::new(1.0_f32, color.gamma_multiply(0.6 * alpha)))
                            .corner_radius(12)
                            .inner_margin(egui::Margin::symmetric(16, 12))
                            .show(ui, |ui| {
                                let glyph = if *ok { icon::CHECK_CIRCLE } else { icon::X_CIRCLE };
                                ui.label(RichText::new(format!("{glyph}  {text}")).color(TEXT.gamma_multiply(alpha)));
                            });
                    });
                ctx.request_repaint_after(Duration::from_millis(50));
            }
        }
    }
}

pub fn run() -> anyhow::Result<()> {
    eframe::run_native(
        "Glorious",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_app_id("glorious-linux")
                .with_title("Glorious")
                .with_inner_size([1120.0, 780.0])
                .with_min_inner_size([860.0, 600.0]),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
