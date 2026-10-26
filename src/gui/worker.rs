//! Background jobs and the messages they send back to the window.
use super::{App, AUTO_REFRESH};
use crate::firmware::{self, FlashEvent};
use crate::gmmk3he;
use crate::settings::{self, LightingSettings, Performance, Settings};
use crate::v3::{Mouse, Snapshot};
use eframe::egui;
use hidapi::HidApi;
use std::sync::mpsc::Sender;
use std::time::Instant;

pub(super) enum Applied {
    Performance(Performance),
    Lighting(LightingSettings),
    Buttons(Vec<(String, String)>),
}

pub(super) enum Msg {
    Snapshot(Result<Box<(Snapshot, Settings)>, String>),
    ActiveStage(Result<u8, String>),
    Applied(Result<Applied, String>),
    FwStatus(Result<(firmware::Status, Vec<&'static str>), String>),
    Fw(FlashEvent),
    /// (mouse on its cable, receiver plugged in, missing tools)
    FwConnections(bool, bool, Vec<&'static str>),
    FwDone(Result<(), String>),
    KbFirmware(Result<Vec<gmmk3he::Component>, String>),
}

impl App {
    fn spawn(&self, ctx: &egui::Context, job: impl FnOnce(&Sender<Msg>, &egui::Context) + Send + 'static) {
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            job(&tx, &ctx);
            ctx.request_repaint();
        });
    }

    pub(super) fn with_mouse<T: Send + 'static>(
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

    pub(super) fn refresh_mouse(&mut self, ctx: &egui::Context) {
        self.with_mouse(ctx, |m| Ok(Box::new((m.snapshot()?, settings::read(m)?))), Msg::Snapshot);
    }

    /// Follow DPI-button presses: ask the mouse which stage it is using.
    pub(super) fn poll_stage(&mut self, ctx: &egui::Context) {
        self.polling_stage = true;
        self.last_stage_poll = Instant::now();
        self.spawn(ctx, |tx, _| {
            let r = HidApi::new()
                .map_err(anyhow::Error::from)
                .and_then(|api| Mouse::open(&api, false)?.active_stage_fast());
            let _ = tx.send(Msg::ActiveStage(r.map_err(|e| format!("{e:#}"))));
        });
    }

    pub(super) fn check_keyboard_firmware(&mut self, ctx: &egui::Context, fw: String) {
        self.kb_fw_busy = true;
        self.spawn(ctx, move |tx, _| {
            let _ = tx.send(Msg::KbFirmware(gmmk3he::compare(&fw).map_err(|e| format!("{e:#}"))));
        });
    }

    pub(super) fn check_firmware(&mut self, ctx: &egui::Context) {
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

    pub(super) fn flash(&mut self, ctx: &egui::Context) {
        let Some(st) = self.fw_status.clone() else { return };
        let (part, force, dry, unverified) = (self.fw_part, self.fw_force, self.fw_dry, self.fw_allow_unverified);
        self.flashing = true;
        self.fw_confirm = false;
        self.fw_log.clear();
        self.fw_step = None;
        self.fw_progress = 0.0;
        self.fw_result = None;
        self.spawn(ctx, move |tx, ctx| {
            let emit = |event: FlashEvent| {
                let _ = tx.send(Msg::Fw(event));
                ctx.request_repaint();
            };
            let r = firmware::flash(&st, part, force, dry, unverified, &emit);
            let _ = tx.send(Msg::FwDone(r.map_err(|e| format!("{e:#}"))));
        });
    }

    /// Watch for the mouse cable and the receiver while the Firmware page is open.
    pub(super) fn check_connections(&mut self, ctx: &egui::Context) {
        self.fw_conn_checking = true;
        self.fw_last_conn = Instant::now();
        self.spawn(ctx, |tx, _| {
            let (cabled, receiver) = firmware::connections();
            let _ = tx.send(Msg::FwConnections(cabled, receiver, firmware::missing_tools(true)));
        });
    }

    pub(super) fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Snapshot(r) => self.on_snapshot(r),
            Msg::ActiveStage(r) => self.on_active_stage(r),
            Msg::Applied(r) => self.on_applied(r),
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
            Msg::Fw(event) => self.on_flash_event(event),
            Msg::FwConnections(cabled, receiver, missing) => {
                self.fw_conn_checking = false;
                (self.fw_cabled, self.fw_receiver, self.fw_missing) = (cabled, receiver, missing);
            }
            Msg::FwDone(r) => self.on_flash_done(r),
            Msg::KbFirmware(r) => {
                self.kb_fw_busy = false;
                if let Ok(c) = r {
                    self.keyboard.components = Some(c);
                }
            }
        }
    }

    fn on_snapshot(&mut self, r: Result<Box<(Snapshot, Settings)>, String>) {
        self.mouse_busy = false;
        let (s, device) = match r {
            Ok(both) => *both,
            Err(e) => {
                self.mouse_error = Some(e);
                return;
            }
        };
        self.mouse_error = None;
        // The mouse is the source of truth; keep edits the user hasn't applied yet.
        let first = !self.settings_loaded;
        if first || self.draft.performance == self.saved.performance {
            self.draft.performance = device.performance.clone();
        }
        if first || self.draft.lighting == self.saved.lighting {
            self.draft.lighting = device.lighting.clone();
        }
        if first || self.draft.buttons == self.saved.buttons {
            self.draft.buttons = device.buttons.clone();
        }
        self.saved = device;
        self.settings_loaded = true;
        self.snapshot = Some(s);
    }

    fn on_active_stage(&mut self, r: Result<u8, String>) {
        self.polling_stage = false;
        // A few stale replies are normal; a long run means the mouse fell asleep.
        self.stage_poll_failures = if r.is_ok() { 0 } else { self.stage_poll_failures + 1 };
        if self.stage_poll_failures >= 12 {
            self.stage_poll_failures = 0;
            self.last_refresh = Instant::now() - AUTO_REFRESH;
        }
        let Ok(stage) = r else { return };
        let stage = stage as usize;
        if self.settings_loaded && stage != self.saved.performance.active {
            if self.draft.performance.active == self.saved.performance.active {
                self.draft.performance.active = stage;
            }
            self.saved.performance.active = stage;
        }
    }

    fn on_applied(&mut self, r: Result<Applied, String>) {
        self.mouse_busy = false;
        let applied = match r {
            Ok(applied) => applied,
            Err(e) => {
                self.toast(format!("Could not apply: {e}"), false);
                self.mouse_error = Some(e);
                return;
            }
        };
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
        self.toast(format!("{what} settings saved to the mouse"), true);
    }

    fn on_flash_event(&mut self, event: FlashEvent) {
        match event {
            FlashEvent::Log(line) => self.fw_log.push(line),
            FlashEvent::Step(n, of, title) => {
                self.fw_log.push(format!("[{n}/{of}] {title}"));
                self.fw_step = Some((n, of, title));
                self.fw_progress = 0.0;
            }
            FlashEvent::Progress(p) => self.fw_progress = p,
        }
    }

    fn on_flash_done(&mut self, r: Result<(), String>) {
        self.flashing = false;
        self.fw_result = Some(r.is_ok());
        match r {
            Ok(()) => self.toast(if self.fw_dry { "Dry run finished" } else { "Firmware update finished" }, true),
            Err(e) => {
                self.fw_log.push(format!("ERROR: {e}"));
                self.toast("Firmware update failed - see the log", false);
            }
        }
    }
}
