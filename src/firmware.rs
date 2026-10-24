//! Firmware check and update for Model O3/D3 Wireless and the V3 receiver.
//!
//! Re-implements CORE 2.1.23's v3-updater (updateMouse.exe / updateDongle.exe); the byte
//! sequences were captured by running those updaters against a mocked node-hid:
//!   mouse    : HID enter-DFU -> nRF52 serial bootloader -> `nrfutil dfu serial`
//!   receiver : HID enter-DFU (nRF52) -> `nrfutil dfu serial` -> HID exit-DFU,
//!              HID enter-DFU (GD32) -> GigaDevice ROM DFU -> `dfu-util`
//! Progress goes through a `log` callback so the CLI and the GUI share this code.
use crate::hid;
use crate::v3::{self, Mouse, MOUSE, RECEIVER, REPORT_ID, REPORT_LEN, VID};
use anyhow::{anyhow, bail, Context, Result};
use hidapi::HidApi;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use std::{fs, thread, thread::sleep};

pub const MANIFEST_URL: &str = "https://gloriouscore.nyc3.digitaloceanspaces.com/CORE2/versions-manifest.prod.json";
const GD32_DFU: &str = "28e9:0189";
const DFU_MAGIC: [u8; 4] = [0x59, 0x4F, 0x4C, 0x4F]; // "YOLO"
const SEL_GD32: u8 = 0x00;
const SEL_NRF52: u8 = 0x01;

pub fn manifest() -> Result<serde_json::Value> {
    ureq::get(MANIFEST_URL).call()?.into_json().context("bad firmware manifest")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Part {
    All,
    Mouse,
    Receiver,
}

/// Installed versus latest firmware.
#[derive(Clone, Debug)]
pub struct Status {
    pub product: &'static str,
    pub color: String,
    wired_pid: u16,
    boot_pid: u16,
    pub mouse: String,
    pub receiver: Option<Vec<String>>,
    pub latest_mouse: String,
    pub latest_receiver: Vec<String>,
    url: String,
}

impl Status {
    pub fn mouse_outdated(&self) -> bool {
        self.mouse != self.latest_mouse
    }
    pub fn receiver_outdated(&self) -> bool {
        self.receiver.as_ref().is_some_and(|r| *r != self.latest_receiver)
    }
    /// Which parts `flash` would touch.
    pub fn plan(&self, part: Part, force: bool) -> (bool, bool) {
        let mouse = matches!(part, Part::All | Part::Mouse) && (self.mouse_outdated() || force);
        let receiver = matches!(part, Part::All | Part::Receiver) && self.receiver.is_some() && (self.receiver_outdated() || force);
        (mouse, receiver)
    }
}

pub fn status() -> Result<Status> {
    let api = HidApi::new()?;
    // Read everything from the mouse first so its lock is released before the network fetch.
    let m = Mouse::open(&api, false)?;
    let name = m.model_name()?;
    let &(_, product, wired_pid, boot_pid) = v3::MODELS
        .iter()
        .find(|x| x.0 == name)
        .ok_or_else(|| anyhow!("unknown mouse model {name:?}"))?;
    let receiver = if m.via_receiver() {
        let mut r = m.receiver_firmware()?.to_vec();
        r.sort();
        Some(r)
    } else {
        None
    };
    let color = m.color()?;
    let mouse = m.firmware()?;
    drop(m);

    // The manifest's version_receiver field is stale, so versions come from the bundle name:
    // `<Model>-<mouse>_<receiver A>-<receiver B>_<dock>.zip`.
    let manifest = manifest()?;
    let entry = manifest["Mouse"]
        .as_array()
        .and_then(|a| a.iter().find(|e| e["productId"] == product))
        .ok_or_else(|| anyhow!("{product} not in firmware manifest"))?;
    let key = if color == "White" { "whiteModelDownloadPath" } else { "downloadPath" };
    let url = entry[key].as_str().ok_or_else(|| anyhow!("no download for {product}"))?.to_string();
    let stem = url.rsplit('/').next().unwrap_or("").trim_end_matches(".zip");
    let parts: Vec<&str> = stem.split_once('-').map(|(_, v)| v).unwrap_or("").split('_').collect();
    if parts.len() < 2 {
        bail!("unexpected firmware bundle name {stem}");
    }
    let mut latest_receiver: Vec<String> = parts[1].split('-').map(String::from).collect();
    latest_receiver.sort();
    let latest_mouse = parts[0].to_string();
    Ok(Status { product, color, wired_pid, boot_pid, mouse, receiver, latest_mouse, latest_receiver, url })
}

pub fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?.to_str()?.split(':').map(|d| Path::new(d).join(name)).find(|p| p.is_file())
}

/// nrfutil 7+ ships the nRF5 SDK DFU code as `nrf5sdk-tools`; 6.x had it at top level.
fn nrfutil() -> Option<Vec<String>> {
    let exe = which("nrfutil")?.to_string_lossy().into_owned();
    let new = Command::new(&exe).args(["nrf5sdk-tools", "dfu", "serial", "--help"]).output();
    Some(if new.map(|o| o.status.success()).unwrap_or(false) { vec![exe, "nrf5sdk-tools".into()] } else { vec![exe] })
}

/// Missing external tools for the given plan, as install hints.
pub fn missing_tools(receiver: bool) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if nrfutil().is_none() {
        missing.push("nrfutil (`paru -S nrfutil`, then `nrfutil install nrf5sdk-tools`)");
    }
    if receiver && which("dfu-util").is_none() {
        missing.push("dfu-util (`sudo pacman -S dfu-util`)");
    }
    missing
}

fn wait_for<T>(what: &str, timeout_s: u64, mut f: impl FnMut() -> Option<T>) -> Result<T> {
    let end = Instant::now() + Duration::from_secs(timeout_s);
    while Instant::now() < end {
        if let Some(v) = f() {
            return Ok(v);
        }
        sleep(Duration::from_millis(500));
    }
    bail!("timed out waiting for {what}")
}

fn present(api: &mut HidApi, pid: u16) -> bool {
    api.refresh_devices().is_ok() && hid::open(api, VID, &[pid], 0xFF00, 0x01).ok().flatten().is_some()
}

struct Flasher<'a> {
    api: HidApi,
    dry: bool,
    log: &'a dyn Fn(String),
}

impl Flasher<'_> {
    /// Fire-and-forget: the device resets right after, so there is no reply.
    fn send_dfu(&mut self, pid: u16, target: u8, payload: &[u8]) -> Result<()> {
        let mut buf = [0u8; REPORT_LEN];
        buf[..4].copy_from_slice(&[REPORT_ID, target, if target == MOUSE { 0x72 } else { 0x70 }, payload.len() as u8]);
        buf[4..4 + payload.len()].copy_from_slice(payload);
        let hex: Vec<String> = buf[..12].iter().map(|b| format!("{b:02x}")).collect();
        self.api.refresh_devices()?;
        let dev = hid::open(&self.api, VID, &[pid], 0xFF00, 0x01)?;
        (self.log)(format!("  HID {} <- {}", dev.as_ref().map_or("(not connected)", |d| d.path.as_str()), hex.join(" ")));
        if self.dry {
            return Ok(());
        }
        let dev = dev.ok_or_else(|| anyhow!("device {VID:04x}:{pid:04x} not found"))?;
        hid::send_feature(&dev.dev, &buf)
    }

    /// Run an external flasher, streaming its output into the log.
    fn run(&self, cmd: &[String]) -> Result<()> {
        (self.log)(format!("  $ {}", cmd.join(" ")));
        if self.dry {
            return Ok(());
        }
        let mut child = Command::new(&cmd[0])
            .args(&cmd[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("cannot run {}", cmd[0]))?;
        let (tx, rx) = mpsc::channel();
        let pipes: [Box<dyn Read + Send>; 2] =
            [Box::new(child.stdout.take().unwrap()), Box::new(child.stderr.take().unwrap())];
        for pipe in pipes {
            let tx = tx.clone();
            thread::spawn(move || {
                for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                    let _ = tx.send(line);
                }
            });
        }
        drop(tx);
        for line in rx {
            (self.log)(format!("    {line}"));
        }
        if !child.wait()?.success() {
            bail!("{} failed - the device stays in its bootloader, so it is safe to run this again", cmd[0]);
        }
        Ok(())
    }

    fn wait_back(&mut self, pid: u16, what: &str) -> Result<()> {
        if !self.dry {
            let api = &mut self.api;
            wait_for(what, 30, || present(api, pid).then_some(()))?;
        }
        Ok(())
    }

    fn port(&self, pid: u16, what: &str) -> Result<String> {
        if self.dry {
            return Ok("/dev/ttyACM?".into());
        }
        wait_for(what, 20, || hid::tty_for(VID, pid))
    }

    fn mouse(&mut self, st: &Status, files: &Files, nrf: &[String]) -> Result<()> {
        (self.log)("[mouse] needs the USB cable plugged into the mouse".into());
        if !self.dry {
            let api = &mut self.api;
            wait_for("the mouse over USB (plug in the cable)", 60, || present(api, st.wired_pid).then_some(()))?;
        }
        self.send_dfu(st.wired_pid, MOUSE, &[&DFU_MAGIC[..], &[0x01]].concat())?;
        let port = self.port(st.boot_pid, "the mouse bootloader")?;
        let mut cmd = nrf.to_vec();
        cmd.extend(["dfu", "serial", "--package", &files.mouse, "--port", &port, "--baud-rate", "115200"].map(String::from));
        self.run(&cmd)
    }

    fn receiver(&mut self, files: &Files, nrf: &[String]) -> Result<()> {
        let dfu = |sel: u8, enter: u8| [&DFU_MAGIC[..], &[sel, enter]].concat();
        (self.log)("[receiver] step 1/2: nRF52 radio".into());
        self.send_dfu(v3::PID_RECEIVER, RECEIVER, &dfu(SEL_NRF52, 1))?;
        let port = self.port(v3::PID_RECEIVER_BOOT, "the receiver nRF bootloader")?;
        let mut cmd = nrf.to_vec();
        cmd.extend(
            ["dfu", "serial", "--package", &files.nrf, "--port", &port, "--flow-control", "0", "--connect-delay", "3",
             "--packet-receipt-notification", "1", "--timeout", "60"]
                .map(String::from),
        );
        self.run(&cmd)?;
        self.wait_back(v3::PID_RECEIVER, "the receiver to come back")?;
        self.send_dfu(v3::PID_RECEIVER, RECEIVER, &dfu(SEL_NRF52, 0))?;
        if !self.dry {
            sleep(Duration::from_secs(3));
        }
        self.wait_back(v3::PID_RECEIVER, "the receiver to come back")?;

        (self.log)("[receiver] step 2/2: GD32 USB controller".into());
        self.send_dfu(v3::PID_RECEIVER, RECEIVER, &dfu(SEL_GD32, 1))?;
        if !self.dry {
            sleep(Duration::from_secs(3));
        }
        self.run(&["dfu-util", "-d", GD32_DFU, "-D", &files.gd32, "-R"].map(String::from))
    }
}

struct Files {
    mouse: String,
    nrf: String,
    gd32: String,
    workdir: PathBuf,
}

impl Drop for Files {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.workdir);
    }
}

fn fetch_bundle(url: &str, log: &dyn Fn(String)) -> Result<Files> {
    log(format!("Downloading {url}"));
    let mut data = Vec::new();
    ureq::get(url).call()?.into_reader().read_to_end(&mut data)?;
    let workdir = std::env::temp_dir().join(format!("glorious-fw-{}", std::process::id()));
    fs::create_dir_all(&workdir)?;
    // extract() rejects entries that would escape workdir.
    zip::ZipArchive::new(std::io::Cursor::new(data))?.extract(&workdir)?;
    let root = fs::read_dir(&workdir)?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_dir() && !p.ends_with("__MACOSX"))
        .ok_or_else(|| anyhow!("empty firmware bundle"))?;
    let path = |rel: &str| root.join(rel).to_string_lossy().into_owned();
    let files = Files {
        mouse: path("mouse/mouse_dfu_package.zip"),
        nrf: path("receiver/release_dongle_nrf.zip"),
        gd32: path("receiver/release_dongle_gd32.bin"),
        workdir,
    };
    for f in [&files.mouse, &files.nrf, &files.gd32] {
        if !Path::new(f).is_file() {
            bail!("firmware bundle is missing {f}");
        }
    }
    Ok(files)
}

/// Flash the parts selected by `st.plan(part, force)`. The caller is responsible for confirmation.
pub fn flash(st: &Status, part: Part, force: bool, dry: bool, log: &dyn Fn(String)) -> Result<()> {
    let (do_mouse, do_recv) = st.plan(part, force);
    if !(do_mouse || do_recv) {
        log("Nothing to flash.".into());
        return Ok(());
    }
    let missing = missing_tools(do_recv);
    if !dry && !missing.is_empty() {
        bail!("missing tools: {}", missing.join(", "));
    }
    let nrf = nrfutil().unwrap_or_else(|| vec!["nrfutil".into()]);
    let files = fetch_bundle(&st.url, log)?;
    let mut f = Flasher { api: HidApi::new()?, dry, log };
    if do_mouse {
        f.mouse(st, &files, &nrf)?;
    }
    if do_recv {
        f.receiver(&files, &nrf)?;
    }
    log(if dry { "Dry run finished - nothing was sent.".into() } else { "Done. Check again to confirm the new versions.".into() });
    Ok(())
}

pub fn cli(flash_requested: bool, part: Part, dry: bool, force: bool) -> Result<()> {
    let st = status()?;
    let mark = |need: bool| if need { "  <- update available" } else { "" };
    println!("{} ({})", st.product, st.color);
    println!("  mouse    : {:<12} latest {}{}", st.mouse, st.latest_mouse, mark(st.mouse_outdated()));
    if let Some(r) = &st.receiver {
        println!("  receiver : {:<12} latest {}{}", r.join("/"), st.latest_receiver.join("/"), mark(st.receiver_outdated()));
    }
    if !flash_requested {
        return Ok(());
    }
    let (m, r) = st.plan(part, force);
    if !dry && (m || r) {
        println!("Do not unplug anything while flashing. Type 'yes' to start:");
        let mut answer = String::new();
        std::io::stdin().lock().read_line(&mut answer)?;
        if answer.trim() != "yes" {
            return Ok(());
        }
    }
    flash(&st, part, force, dry, &|line| println!("{line}"))
}
