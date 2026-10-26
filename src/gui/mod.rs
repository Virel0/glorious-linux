//! egui front end. All HID and network work runs on worker threads that report back over a
//! channel, so the window never blocks on the mouse's 30-200 ms report delays.
mod buttons;
mod format;
mod lighting;
mod overview;
mod performance;
mod sidebar;
mod updates;
mod worker;

use crate::firmware::{self, Part};
use crate::icons as icon;
use crate::kb_gui::{Keyboard, Page};
use crate::settings::{self, Settings};
use crate::theme::{self, *};
use crate::v3::{key_names, Snapshot};
use eframe::egui::{self, RichText, Stroke, Ui};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};
use worker::{Applied, Msg};

const AUTO_REFRESH: Duration = Duration::from_secs(60);
const ASLEEP_RETRY: Duration = Duration::from_secs(3);
const STAGE_POLL: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Mouse,
    Performance,
    Lighting,
    Buttons,
    Kb(Page),
    Firmware,
}

pub struct App {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    tab: Tab,
    mouse_busy: bool,
    last_refresh: Instant,
    /// Live DPI-stage polling (separate from `mouse_busy` so it never greys out the UI).
    polling_stage: bool,
    stage_poll_failures: u32,
    last_stage_poll: Instant,
    /// False until the first successful read, so the editors don't show placeholder values.
    settings_loaded: bool,
    snapshot: Option<Snapshot>,
    mouse_error: Option<String>,
    /// Last values known to be on the mouse, and the values being edited.
    saved: Settings,
    draft: Settings,
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
    /// The user accepted flashing a bundle whose checksum this version doesn't know.
    fw_allow_unverified: bool,
    flashing: bool,
    /// Current flash step (number, total, title) and its progress.
    fw_step: Option<(usize, usize, String)>,
    fw_progress: f32,
    fw_result: Option<bool>,
    fw_cabled: bool,
    fw_receiver: bool,
    fw_conn_checking: bool,
    fw_last_conn: Instant,
    keyboard: Keyboard,
    kb_fw_busy: bool,
    keys: Vec<(String, u8)>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> App {
        theme::install(&cc.egui_ctx);
        let (tx, rx) = channel();
        let saved = Settings::default();
        let mut app = App {
            tx,
            rx,
            tab: Tab::Mouse,
            mouse_busy: false,
            last_refresh: Instant::now(),
            polling_stage: false,
            stage_poll_failures: 0,
            last_stage_poll: Instant::now(),
            settings_loaded: false,
            snapshot: None,
            mouse_error: None,
            draft: saved.clone(),
            saved,
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
            fw_allow_unverified: false,
            flashing: false,
            fw_step: None,
            fw_progress: 0.0,
            fw_result: None,
            fw_cabled: false,
            fw_receiver: false,
            fw_conn_checking: false,
            fw_last_conn: Instant::now() - Duration::from_secs(60),
            keyboard: Keyboard::new(),
            kb_fw_busy: false,
            keys: key_names(),
        };
        app.refresh_mouse(&cc.egui_ctx);
        app
    }

    fn toast(&mut self, text: impl Into<String>, ok: bool) {
        self.toast = Some((text.into(), ok, Instant::now()));
    }

    /// Whether the current tab has unapplied edits.
    fn dirty(&self) -> bool {
        match self.tab {
            Tab::Kb(page) => self.keyboard.dirty(page),
            Tab::Performance => self.draft.performance != self.saved.performance,
            Tab::Lighting => self.draft.lighting != self.saved.lighting,
            Tab::Buttons => self.draft.buttons != self.saved.buttons,
            _ => false,
        }
    }

    fn busy(&self) -> bool {
        match self.tab {
            Tab::Kb(_) => self.keyboard.busy || !self.keyboard.connected,
            _ => self.mouse_busy || self.flashing,
        }
    }

    fn apply(&mut self, ctx: &egui::Context) {
        match self.tab {
            Tab::Kb(page) => {
                if let Err(e) = self.keyboard.apply(page) {
                    self.toast(format!("Could not apply: {e:#}"), false);
                }
            }
            Tab::Performance => {
                let (p, prev) = (self.draft.performance.clone(), self.saved.performance.clone());
                if !p.stages.get(p.active).is_some_and(|s| s.enabled) {
                    self.toast("The active DPI stage must be switched on", false);
                    return;
                }
                self.with_mouse(
                    ctx,
                    move |m| settings::apply_performance(m, &p, &prev).map(|_| p),
                    |r| Msg::Applied(r.map(Applied::Performance)),
                );
            }
            Tab::Lighting => {
                let l = self.draft.lighting.clone();
                self.with_mouse(
                    ctx,
                    move |m| settings::apply_lighting(m, &l).map(|_| l),
                    |r| Msg::Applied(r.map(Applied::Lighting)),
                );
            }
            Tab::Buttons => {
                let b = self.draft.buttons.clone();
                self.with_mouse(
                    ctx,
                    move |m| settings::apply_buttons(m, &b).map(|_| b),
                    |r| Msg::Applied(r.map(Applied::Buttons)),
                );
            }
            _ => {}
        }
    }

    fn revert(&mut self) {
        match self.tab {
            Tab::Kb(page) => self.keyboard.revert(page),
            Tab::Performance => self.draft.performance = self.saved.performance.clone(),
            Tab::Lighting => self.draft.lighting = self.saved.lighting.clone(),
            Tab::Buttons => self.draft.buttons = self.saved.buttons.clone(),
            _ => {}
        }
    }

    /// Drain worker messages and start whatever background work is due this frame.
    fn background_work(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            self.handle(msg);
        }
        for (text, ok) in self.keyboard.poll(self.tab == Tab::Kb(Page::Actuation)) {
            self.toast(text, ok);
        }
        if let (Some(fw), None, false) = (self.keyboard.fw.clone(), &self.keyboard.components, self.kb_fw_busy) {
            self.check_keyboard_firmware(ctx, fw);
        }
        // Keep the battery fresh, and reconnect on its own once a sleeping mouse wakes up.
        let wait = if self.mouse_error.is_some() { ASLEEP_RETRY } else { AUTO_REFRESH };
        if !self.mouse_busy && !self.flashing && self.last_refresh.elapsed() >= wait {
            self.refresh_mouse(ctx);
        }
        let live_tab = matches!(self.tab, Tab::Mouse | Tab::Performance);
        let mouse_idle = self.mouse_error.is_none() && !self.mouse_busy && !self.flashing;
        if live_tab
            && self.settings_loaded
            && mouse_idle
            && !self.polling_stage
            && self.last_stage_poll.elapsed() >= STAGE_POLL
        {
            self.poll_stage(ctx);
        }
        ctx.request_repaint_after(if live_tab { STAGE_POLL } else { Duration::from_secs(1) });
        if self.tab != Tab::Firmware {
            return;
        }
        if self.fw_status.is_none() && self.fw_error.is_none() && !self.fw_busy {
            self.check_firmware(ctx);
        }
        if !self.flashing && !self.fw_conn_checking && self.fw_last_conn.elapsed() >= Duration::from_millis(1500) {
            self.check_connections(ctx);
        }
    }

    /// "Unsaved changes" bar with Apply and Revert.
    fn save_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(icon::PENCIL_SIMPLE).size(16.0).color(ACCENT));
            ui.label(RichText::new("Unsaved changes").color(TEXT));
            ui.label(RichText::new("Ctrl+S").size(12.0).color(FAINT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let target = if matches!(self.tab, Tab::Kb(_)) { "Apply to keyboard" } else { "Apply to mouse" };
                let busy = self.busy();
                if primary(ui, !busy, target) {
                    self.apply(ui.ctx());
                }
                if ui.add_enabled(!busy, egui::Button::new("Revert").min_size(egui::vec2(90.0, 34.0))).clicked() {
                    self.revert();
                }
                if let (Tab::Kb(_), Some((i, n))) = (self.tab, self.keyboard.progress()) {
                    ui.add(egui::ProgressBar::new(i as f32 / n as f32).desired_width(140.0));
                } else if busy {
                    ui.spinner();
                }
            });
        });
    }

    fn page(&mut self, ui: &mut Ui) {
        match self.tab {
            Tab::Mouse => self.overview_tab(ui),
            Tab::Performance => self.performance_tab(ui),
            Tab::Lighting => self.lighting_tab(ui),
            Tab::Buttons => self.buttons_tab(ui),
            Tab::Firmware => self.firmware_tab(ui),
            Tab::Kb(Page::Home) => self.keyboard.home(ui),
            Tab::Kb(Page::Lighting) => self.keyboard.lighting(ui),
            Tab::Kb(Page::Actuation) => self.keyboard.actuation(ui),
            Tab::Kb(Page::Keymap) => self.keyboard.keymap(ui),
        }
    }

    /// The last toast, fading out over its final second.
    fn show_toast(&self, ctx: &egui::Context) {
        let Some((text, ok, at)) = &self.toast else { return };
        let age = at.elapsed().as_secs_f32();
        if age >= 4.0 {
            return;
        }
        let alpha = (4.0 - age).min(1.0);
        egui::Area::new(egui::Id::unique("toast"))
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

impl eframe::App for App {
    fn ui(&mut self, root: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = &root.ctx().clone();
        self.background_work(ctx);
        if self.dirty() && !self.busy() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            self.apply(ctx);
        }

        egui::Panel::left("nav")
            .exact_size(220.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin { left: 12, right: 12, top: 0, bottom: 0 }))
            .show(root, |ui| self.sidebar(ui));

        if self.dirty() {
            egui::Panel::bottom("savebar")
                .frame(
                    egui::Frame::new()
                        .fill(SURFACE)
                        .stroke(Stroke::new(1.0_f32, BORDER))
                        .inner_margin(egui::Margin::symmetric(24, 12)),
                )
                .show(root, |ui| self.save_bar(ui));
        }

        egui::CentralPanel::no_frame()
            .frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin { left: 32, right: 32, top: 28, bottom: 0 }))
            .show(root, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_max_width(ui.available_width().min(980.0));
                    self.page(ui);
                    ui.add_space(24.0);
                });
            });

        self.show_toast(ctx);
    }
}

pub fn run() -> anyhow::Result<()> {
    eframe::run_native(
        "Glorious",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_app_id("io.github.virel0.GloriousLinux")
                .with_icon(std::sync::Arc::new(
                    eframe::icon_data::from_png_bytes(include_bytes!(
                        "../../packaging/icons/hicolor/256x256/apps/io.github.virel0.GloriousLinux.png"
                    ))
                    .unwrap_or_default(),
                ))
                .with_title("Glorious")
                .with_inner_size([1120.0, 780.0])
                .with_min_inner_size([860.0, 600.0]),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
