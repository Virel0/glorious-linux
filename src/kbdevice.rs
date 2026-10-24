//! Keyboard I/O: transport and worker thread for the GMMK 3 HE.
//!
//! Adapted from inglorious-core-linux src/device.rs (MIT, Copyright (c) 2026 thomas griffin).
//! Change: the keyboard's input interface (0xFFFF/0x01) is optional. Besides the live key-travel
//! reports it carries every keystroke, so the udev rule no longer grants access to it and only
//! the vendor settings interface (0xFFFF/0x02) is required. Live travel works when the user
//! chooses to grant that interface too.
use crate::magneton::protocol::{self as p, Report};
use crate::magneton::state::State;
use anyhow::{bail, ensure, Context, Result};
use hidapi::{HidApi, HidDevice};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

/// Where the last applied keyboard settings live; shared with the inglorious-core-linux app.
pub fn state_path() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .context("cannot determine configuration directory")?;
    Ok(base.join("glorious-core-rs/state.json"))
}

pub fn load_state() -> Result<State> {
    let path = state_path()?;
    let mut s: State = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("invalid state {} (left untouched)", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => State::default(),
        Err(e) => return Err(e.into()),
    };
    s.validate()?;
    s.profiles[s.profile as usize].selected_layer = s.layer;
    Ok(s)
}

/// Keep a timestamped backup of the previous file and replace it atomically.
fn save_state_to(s: &State, path: &Path) -> Result<()> {
    use std::io::Write;
    s.validate()?;
    std::fs::create_dir_all(path.parent().context("state path has no parent")?)?;
    if path.exists() {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
        std::fs::copy(path, path.with_extension(format!("json.{stamp}.bak")))?;
    }
    let tmp = path.with_extension("json.tmp");
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(&serde_json::to_vec_pretty(s)?)?;
    file.sync_all()?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

const VID: u16 = 0x342d;
const PID: u16 = 0xe3de;
const SEND_WAIT: Duration = Duration::from_millis(550);
const GET_WAIT: Duration = Duration::from_millis(150);
const KEYSTROKE_WAIT: Duration = Duration::from_millis(650);
const COLOR_START_WAIT: Duration = Duration::from_millis(750);
/// Per-page waits the original passes for per-key colour (0x0C) and mode (0x51) pages.
const COLOR_PAGE_WAIT: Duration = Duration::from_millis(150);
const MODE_PAGE_WAIT: Duration = Duration::from_millis(100);

pub struct Transport {
    /// Vendor feature port (usage 0xFFFF/0x02): all settings.
    cfg: HidDevice,
    /// Input report 5 channel (0xFFFF/0x01), only for live travel; None without permission.
    events: Option<HidDevice>,
}

impl Transport {
    pub fn open() -> Result<Self> {
        let api = HidApi::new()?;
        let find = |usage: u16| -> Result<Option<hidapi::DeviceInfo>> {
            let matches: Vec<_> = api
                .device_list()
                .filter(|d| {
                    d.vendor_id() == VID && d.product_id() == PID && d.usage_page() == 0xffff && d.usage() == usage
                })
                .cloned()
                .collect();
            match matches.len() {
                0 => Ok(None),
                1 => Ok(matches.into_iter().next()),
                n => bail!("{n} GMMK 3 HE keyboards found; unplug all but one"),
            }
        };
        let cfg_info = find(0x02)?.context("keyboard not found; is it plugged in?")?;
        let path = cfg_info.path().to_string_lossy().into_owned();
        let cfg = cfg_info.open_device(&api).with_context(|| {
            format!("cannot open {path}; install the glorious-linux udev rule and replug the keyboard")
        })?;
        let events = find(0x01)?.and_then(|i| i.open_device(&api).ok());
        if let Some(e) = &events {
            e.set_blocking_mode(false)?;
        }
        Ok(Self { cfg, events })
    }

    pub fn live_available(&self) -> bool {
        self.events.is_some()
    }

    pub fn send(&self, r: &Report) -> Result<()> {
        let mut wire = [0; 65];
        wire[1..].copy_from_slice(r);
        self.cfg.send_feature_report(&wire)?;
        thread::sleep(match r[0] {
            19 => KEYSTROKE_WAIT,
            12 => COLOR_PAGE_WAIT,
            81 => MODE_PAGE_WAIT,
            _ => SEND_WAIT,
        });
        Ok(())
    }

    pub fn query(&self, r: &Report) -> Result<Report> {
        self.send(r)?;
        let mut wire = [0; 65];
        let n = self.cfg.get_feature_report(&mut wire)?;
        thread::sleep(GET_WAIT);
        ensure!(n == 65, "short feature response: {n}");
        ensure!(wire[1] == r[0], "unexpected response opcode {:02x}", wire[1]);
        let mut out = [0; 64];
        out.copy_from_slice(&wire[1..]);
        Ok(out)
    }

    /// Newest travel sample waiting on the input interface, if any.
    fn live(&self) -> Result<Option<f32>> {
        let Some(events) = &self.events else { return Ok(None) };
        let mut r = [0; 65];
        let mut latest = None;
        // Catch up to current travel without letting continuous input starve commands.
        for _ in 0..256 {
            let n = events.read_timeout(&mut r, 0)?;
            if n == 0 {
                break;
            }
            if let Some(value) = p::parse_visualization(&r[..n]) {
                latest = Some(value);
            }
        }
        Ok(latest)
    }
}

#[derive(Debug)]
pub enum Command {
    Shutdown,
    Info,
    Write { reports: Vec<Report>, state: Option<Box<State>> },
    Live(bool),
}

#[derive(Debug)]
pub enum Event {
    Info { fw: String, profile: u8, layer: u8, sleep: Option<u16>, live_available: bool },
    Progress(usize, usize),
    Saved(Box<State>),
    ActiveSlot { profile: u8, layer: u8 },
    Live(f32),
    Done,
    Error(String),
}

pub fn info(t: &Transport) -> Result<Event> {
    let mut fw = Vec::new();
    for op in [0xf0, 0x80, 0xae] {
        fw.push(p::parse_fw(&t.query(&p::get_fw(op)?)?)?);
    }
    let (profile, layer) = p::parse_profile(&t.query(&p::get_profile())?)?;
    let sleep = p::parse_sleep(&t.query(&p::get_sleep())?)?;
    Ok(Event::Info { fw: fw.join(" / "), profile, layer, sleep, live_available: t.live_available() })
}

/// The worker owns the keyboard; the GUI talks to it through `Command`s and gets `Event`s back.
pub fn worker() -> (Sender<Command>, Receiver<Event>) {
    let (tx, rx) = mpsc::channel();
    let (et, er) = mpsc::channel();
    thread::spawn(move || {
        let mut transport: Option<Transport> = None;
        let mut live = false;
        loop {
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(Command::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if let (true, Some(t)) = (live, transport.as_ref()) {
                        let _ = t.send(&p::visualization(false));
                    }
                    break;
                }
                Ok(cmd) => {
                    let writing = matches!(cmd, Command::Write { .. });
                    let result = (|| -> Result<()> {
                        if transport.is_none() {
                            transport = Some(Transport::open()?);
                        }
                        let t = transport.as_ref().unwrap();
                        match cmd {
                            Command::Shutdown => unreachable!("handled above"),
                            Command::Info => {
                                let _ = et.send(info(t)?);
                            }
                            Command::Live(enabled) => {
                                ensure!(
                                    !enabled || t.live_available(),
                                    "live travel needs access to the keyboard's input interface"
                                );
                                if enabled {
                                    t.live()?; // Discard packets left over from a previous live session.
                                }
                                t.send(&p::visualization(enabled))?;
                                live = enabled;
                            }
                            Command::Write { mut reports, state } => {
                                ensure!(!reports.is_empty(), "empty write");
                                if let Some(s) = state.as_ref() {
                                    s.validate()?;
                                    // Un-indexed lighting/performance/mode writes must target the selected profile.
                                    if reports[0][0] != 5 {
                                        reports.insert(0, p::set_profile(s.profile, s.layer)?);
                                    }
                                }
                                if reports.iter().any(|r| r[0] == 12) {
                                    thread::sleep(COLOR_START_WAIT);
                                }
                                let n = reports.len();
                                for (i, r) in reports.iter().enumerate() {
                                    t.send(r)?;
                                    let _ = et.send(Event::Progress(i + 1, n));
                                }
                                if reports[0][0] == 5 {
                                    let _ = et.send(Event::ActiveSlot { profile: reports[0][1], layer: reports[0][2] });
                                }
                                if let Some(s) = state {
                                    save_state_to(&s, &state_path()?)?;
                                    let _ = et.send(Event::Saved(s));
                                }
                            }
                        }
                        Ok(())
                    })();
                    match result {
                        Ok(()) => {
                            let _ = et.send(Event::Done);
                        }
                        Err(e) => {
                            // Reopen on the next explicit action; never replay a failed batch.
                            transport = None;
                            live = false;
                            let note = if writing {
                                ". A partial batch may have reached the board; saved state was not advanced."
                            } else {
                                ""
                            };
                            let _ = et.send(Event::Error(format!("{e:#}{note}")));
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if live {
                if let Some(t) = transport.as_ref() {
                    match t.live() {
                        Ok(Some(v)) => {
                            let _ = et.send(Event::Live(v));
                        }
                        Ok(None) => {}
                        Err(e) => {
                            transport = None;
                            live = false;
                            let _ = et.send(Event::Error(format!("Live read stopped: {e}")));
                        }
                    }
                }
            }
        }
    });
    (tx, er)
}
