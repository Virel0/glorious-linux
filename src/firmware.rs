//! Firmware check and update for Model O3/D3 Wireless and the V3 receiver.
//!
//! Re-implements CORE 2.1.23's v3-updater (updateMouse.exe / updateDongle.exe); the byte
//! sequences were captured by running those updaters against a mocked node-hid:
//!   mouse    : HID enter-DFU -> nRF52 serial bootloader -> Nordic serial DFU (nordic_dfu.rs)
//!   receiver : HID enter-DFU (nRF52) -> Nordic serial DFU -> HID exit-DFU,
//!              HID enter-DFU (GD32) -> GigaDevice ROM DFU -> `dfu-util`
//! Progress goes out as `FlashEvent`s so the CLI and the GUI share this code.
use crate::v3::{self, Mouse, MOUSE, RECEIVER, REPORT_ID, REPORT_LEN, VID};
use crate::{hid, nordic_dfu};
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
/// Firmware bundles are ~200 KB; refuse anything absurd.
const MAX_BUNDLE_BYTES: u64 = 32 * 1024 * 1024;

/// SHA-256 of the Glorious firmware bundles this version has been checked against. The nRF52
/// images inside are signed and verified by the devices themselves, but the receiver's GD32
/// image is not, so the whole bundle is pinned. A bundle with a known name and a different hash
/// is refused; an unknown (newer) bundle needs explicit confirmation.
#[rustfmt::skip]
const KNOWN_BUNDLES: [(&str, &str); 4] = [
    ("Model_D3_Wireless_Black-1.3.3_1.1.0-1.3.0_1.0.0.zip", "b4142ab4705a9a9872bdb51b8aea5af78de1512d5867776f2d59a4b00ea30038"),
    ("Model_D3_Wireless_White-1.3.3_1.1.0-1.3.0_1.0.0.zip", "da58c82303e89dbea300c60b808fa0ad041153681e5ec3e68b4fb89bb22a4993"),
    ("Model_O3_Wireless_Black-1.3.3_1.1.0-1.3.0_1.0.0.zip", "470ed85d60dc9e6ef8026af72ff561a45182f3ad47d145670e779cfaf941204f"),
    ("Model_O3_Wireless_White-1.3.3_1.1.0-1.3.0_1.0.0.zip", "542e796ecb375957be4285705c9d4f757407fd489ac559dc564149fd520ac9c1"),
];

fn bundle_name(url: &str) -> &str {
    url.rsplit('/').next().unwrap_or(url)
}

fn known_hash(url: &str) -> Option<&'static str> {
    KNOWN_BUNDLES.iter().find(|(name, _)| *name == bundle_name(url)).map(|(_, hash)| *hash)
}

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
    /// Whether the bundle to download is one whose checksum this version knows.
    pub fn bundle_verified(&self) -> bool {
        known_hash(&self.url).is_some()
    }
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
        let receiver = matches!(part, Part::All | Part::Receiver)
            && self.receiver.is_some()
            && (self.receiver_outdated() || force);
        (mouse, receiver)
    }
}

pub fn status() -> Result<Status> {
    let api = HidApi::new()?;
    // Read everything from the mouse first so its lock is released before the network fetch.
    let m = Mouse::open(&api, false)?;
    let name = m.model_name()?;
    let &(_, product, wired_pid, boot_pid) =
        v3::MODELS.iter().find(|x| x.0 == name).ok_or_else(|| anyhow!("unknown mouse model {name:?}"))?;
    let mut receiver = if m.via_receiver() { Some(m.receiver_firmware()?.to_vec()) } else { None };
    let color = m.color()?;
    let mouse = m.firmware()?;
    drop(m);
    // With the mouse on its cable the receiver is a separate device; it still answers for itself.
    if receiver.is_none() {
        receiver = Mouse::open_pids(&api, &[v3::PID_RECEIVER], false)
            .ok()
            .and_then(|r| r.receiver_firmware().ok())
            .map(|r| r.to_vec());
    }
    if let Some(r) = receiver.as_mut() {
        r.sort();
    }

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

/// Missing external tools for the given plan, as install hints. The nRF52 parts are flashed by
/// the built-in Nordic DFU; only the receiver's GD32 chip needs dfu-util.
pub fn missing_tools(receiver: bool) -> Vec<&'static str> {
    let mut missing = Vec::new();
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

/// What is plugged in right now: (mouse on its USB cable, receiver).
pub fn connections() -> (bool, bool) {
    let Ok(api) = HidApi::new() else { return (false, false) };
    let has = |pid: u16| api.device_list().any(|d| d.vendor_id() == VID && d.product_id() == pid);
    (v3::MODELS.iter().any(|m| has(m.2)), has(v3::PID_RECEIVER))
}

/// Progress of a flash, for the CLI and the GUI.
#[derive(Clone, Debug)]
pub enum FlashEvent {
    Log(String),
    /// A new step started: (1-based number, total, title).
    Step(usize, usize, String),
    /// Progress of the current step, 0..=1.
    Progress(f32),
}

/// Pull "NN%" progress out of dfu-util's "Download [=====    ]  45%" lines.
fn dfu_util_percent(line: &str) -> Option<f32> {
    if !(line.contains("Download") || line.contains("Erase")) || !line.contains('%') {
        return None;
    }
    let pct = line.split('%').next()?.rsplit(|c: char| !c.is_ascii_digit()).next()?;
    pct.parse::<f32>().ok().map(|p| (p / 100.0).clamp(0.0, 1.0))
}

struct Flasher<'a> {
    api: HidApi,
    dry: bool,
    emit: &'a dyn Fn(FlashEvent),
    step: usize,
    steps: usize,
}

impl Flasher<'_> {
    fn log(&self, line: String) {
        (self.emit)(FlashEvent::Log(line));
    }

    fn begin(&mut self, title: &str) {
        self.step += 1;
        (self.emit)(FlashEvent::Step(self.step, self.steps, title.to_string()));
        (self.emit)(FlashEvent::Progress(0.0));
    }

    /// Fire-and-forget: the device resets right after, so there is no reply.
    fn send_dfu(&mut self, pid: u16, target: u8, payload: &[u8]) -> Result<()> {
        let mut buf = [0u8; REPORT_LEN];
        buf[..4].copy_from_slice(&[REPORT_ID, target, if target == MOUSE { 0x72 } else { 0x70 }, payload.len() as u8]);
        buf[4..4 + payload.len()].copy_from_slice(payload);
        let hex: Vec<String> = buf[..12].iter().map(|b| format!("{b:02x}")).collect();
        self.api.refresh_devices()?;
        let dev = hid::open(&self.api, VID, &[pid], 0xFF00, 0x01)?;
        self.log(format!("  HID {} <- {}", dev.as_ref().map_or("(not connected)", |d| d.path.as_str()), hex.join(" ")));
        if self.dry {
            return Ok(());
        }
        let dev = dev.ok_or_else(|| anyhow!("device {VID:04x}:{pid:04x} not found"))?;
        hid::send_feature(&dev.dev, &buf)
    }

    /// Run an external flasher, streaming its output into the log and its percentage into
    /// the progress bar (dfu-util redraws its bar with carriage returns).
    fn run(&self, cmd: &[String]) -> Result<()> {
        self.log(format!("  $ {}", cmd.join(" ")));
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
                for chunk in BufReader::new(pipe).split(b'\n').map_while(Result::ok) {
                    for piece in chunk.split(|&b| b == b'\r') {
                        let _ = tx.send(String::from_utf8_lossy(piece).trim_end().to_string());
                    }
                }
            });
        }
        drop(tx);
        let mut last_bar = String::new();
        for line in rx.into_iter().filter(|l| !l.is_empty()) {
            match dfu_util_percent(&line) {
                Some(p) => {
                    (self.emit)(FlashEvent::Progress(p));
                    last_bar = line;
                }
                None => self.log(format!("    {line}")),
            }
        }
        if !last_bar.is_empty() {
            self.log(format!("    {last_bar}"));
        }
        if !child.wait()?.success() {
            bail!("{} failed - the device stays in its bootloader, so it is safe to run this again", cmd[0]);
        }
        (self.emit)(FlashEvent::Progress(1.0));
        Ok(())
    }

    fn wait_back(&mut self, pid: u16, what: &str) -> Result<()> {
        if !self.dry {
            let api = &mut self.api;
            wait_for(what, 30, || present(api, pid).then_some(()))?;
        }
        Ok(())
    }

    /// Flash a Nordic DFU package through the serial bootloader that appeared on `port`.
    fn nordic(&self, port: &str, package: &str, opts: nordic_dfu::Options) -> Result<()> {
        let pkg = nordic_dfu::Package::load(Path::new(package))?;
        let name = Path::new(package).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.log(format!(
            "  Nordic DFU {name} via {port}: init packet {} bytes, firmware {} bytes, receipt every {} packets",
            pkg.init.len(),
            pkg.firmware.len(),
            opts.prn
        ));
        if self.dry {
            (self.emit)(FlashEvent::Progress(1.0));
            return Ok(());
        }
        let log = |line: String| self.log(line);
        let progress = |p: f32| (self.emit)(FlashEvent::Progress(p));
        nordic_dfu::flash(port, Path::new(package), opts, &log, &progress)
            .context("firmware transfer failed - the device stays in its bootloader, so it is safe to run this again")
    }

    fn port(&self, pid: u16, what: &str) -> Result<String> {
        if self.dry {
            return Ok("/dev/ttyACM?".into());
        }
        wait_for(what, 20, || hid::tty_for(VID, pid))
    }

    fn mouse(&mut self, st: &Status, files: &Files) -> Result<()> {
        self.begin("Mouse");
        self.send_dfu(st.wired_pid, MOUSE, &[&DFU_MAGIC[..], &[0x01]].concat())?;
        let port = self.port(st.boot_pid, "the mouse bootloader")?;
        // CORE runs `nrfutil dfu serial --baud-rate 115200` with nrfutil's defaults otherwise.
        let opts =
            nordic_dfu::Options { prn: 0, connect_delay: Duration::from_secs(3), timeout: Duration::from_secs(30) };
        self.nordic(&port, &files.mouse, opts)
    }

    fn receiver(&mut self, files: &Files) -> Result<()> {
        let dfu = |sel: u8, enter: u8| [&DFU_MAGIC[..], &[sel, enter]].concat();
        self.begin("Receiver radio (nRF52)");
        self.send_dfu(v3::PID_RECEIVER, RECEIVER, &dfu(SEL_NRF52, 1))?;
        let port = self.port(v3::PID_RECEIVER_BOOT, "the receiver nRF bootloader")?;
        // Same settings CORE passes to nrfutil for the receiver: a receipt after every packet,
        // which paces the transfer through the receiver's USB-to-UART bridge.
        let opts =
            nordic_dfu::Options { prn: 1, connect_delay: Duration::from_secs(3), timeout: Duration::from_secs(60) };
        self.nordic(&port, &files.nrf, opts)?;
        self.wait_back(v3::PID_RECEIVER, "the receiver to come back")?;
        self.send_dfu(v3::PID_RECEIVER, RECEIVER, &dfu(SEL_NRF52, 0))?;
        if !self.dry {
            sleep(Duration::from_secs(3));
        }
        self.wait_back(v3::PID_RECEIVER, "the receiver to come back")?;

        self.begin("Receiver USB chip (GD32)");
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
    /// Private (0700, random name) and removed when dropped.
    _workdir: tempfile::TempDir,
}

fn fetch_bundle(url: &str, allow_unverified: bool, emit: &dyn Fn(FlashEvent)) -> Result<Files> {
    use sha2::{Digest, Sha256};
    emit(FlashEvent::Log(format!("Downloading {url}")));
    let mut data = Vec::new();
    ureq::get(url).call()?.into_reader().take(MAX_BUNDLE_BYTES + 1).read_to_end(&mut data)?;
    if data.len() as u64 > MAX_BUNDLE_BYTES {
        bail!("firmware download is larger than {} MB - refusing it", MAX_BUNDLE_BYTES / 1024 / 1024);
    }
    let digest: String = Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect();
    match known_hash(url) {
        Some(expected) if expected == digest => emit(FlashEvent::Log(format!("  checksum verified (sha256 {})", &digest[..16]))),
        Some(expected) => bail!(
            "the downloaded firmware does not match the known checksum for {} (got {digest}, expected {expected}) - refusing to flash it",
            bundle_name(url)
        ),
        None if allow_unverified => emit(FlashEvent::Log(format!("  unverified bundle, used as confirmed (sha256 {digest})"))),
        None => bail!("{} is newer than this version of glorious-linux knows, so its checksum can't be verified", bundle_name(url)),
    }
    let workdir = tempfile::Builder::new()
        .prefix("glorious-fw-")
        .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .tempdir()?;
    // Only the three files we need are read from the archive and written under fixed names; nothing
    // else in it (paths, symlinks, permissions) ever reaches the disk.
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data))?;
    let mut take = |suffix: &str, name: &str| -> Result<String> {
        let index = (0..archive.len())
            .find(|&i| {
                archive.by_index(i).is_ok_and(|f| {
                    f.is_file() && f.name().is_ok_and(|n| !n.starts_with("__MACOSX") && n.ends_with(suffix))
                })
            })
            .ok_or_else(|| anyhow!("firmware bundle is missing {suffix}"))?;
        let mut entry = archive.by_index(index)?;
        let mut bytes = Vec::new();
        entry.by_ref().take(MAX_BUNDLE_BYTES).read_to_end(&mut bytes)?;
        let path = workdir.path().join(name);
        fs::write(&path, bytes)?;
        Ok(path.to_string_lossy().into_owned())
    };
    let mouse = take("/mouse/mouse_dfu_package.zip", "mouse_dfu_package.zip")?;
    let nrf = take("/receiver/release_dongle_nrf.zip", "release_dongle_nrf.zip")?;
    let gd32 = take("/receiver/release_dongle_gd32.bin", "release_dongle_gd32.bin")?;
    let files = Files { mouse, nrf, gd32, _workdir: workdir };
    Ok(files)
}

/// Why `flash` would refuse right now, if it would.
pub fn blocker(st: &Status, part: Part, force: bool, dry: bool) -> Option<String> {
    let (do_mouse, do_recv) = st.plan(part, force);
    if !(do_mouse || do_recv) {
        return Some("Everything is up to date - nothing to flash.".into());
    }
    if dry {
        return None;
    }
    let (cabled, receiver) = connections();
    if do_mouse && !cabled {
        return Some("Plug the USB cable into the mouse - mouse updates only run over the cable.".into());
    }
    if do_recv && !receiver {
        return Some("Plug in the wireless receiver to update it.".into());
    }
    let missing = missing_tools(do_recv);
    if !missing.is_empty() {
        return Some(format!("Missing {}", missing.join(", ")));
    }
    None
}

/// Flash the parts selected by `st.plan(part, force)`. The caller is responsible for confirmation.
/// Refuses up front (without touching anything) when a needed device or tool is missing.
pub fn flash(
    st: &Status,
    part: Part,
    force: bool,
    dry: bool,
    allow_unverified: bool,
    emit: &dyn Fn(FlashEvent),
) -> Result<()> {
    if let Some(reason) = blocker(st, part, force, dry) {
        bail!("{reason}");
    }
    let (do_mouse, do_recv) = st.plan(part, force);
    let files = fetch_bundle(&st.url, allow_unverified || dry, emit)?;
    let steps = usize::from(do_mouse) + if do_recv { 2 } else { 0 };
    let mut f = Flasher { api: HidApi::new()?, dry, emit, step: 0, steps };
    if do_mouse {
        f.mouse(st, &files)?;
    }
    if do_recv {
        f.receiver(&files)?;
    }
    f.log(if dry {
        "Dry run finished - nothing was sent.".into()
    } else {
        "Done. Check again to confirm the new versions.".into()
    });
    Ok(())
}

pub fn cli(flash_requested: bool, part: Part, dry: bool, force: bool, allow_unverified: bool) -> Result<()> {
    let st = status()?;
    let mark = |need: bool| if need { "  <- update available" } else { "" };
    println!("{} ({})", st.product, st.color);
    println!("  mouse    : {:<12} latest {}{}", st.mouse, st.latest_mouse, mark(st.mouse_outdated()));
    if let Some(r) = &st.receiver {
        println!(
            "  receiver : {:<12} latest {}{}",
            r.join("/"),
            st.latest_receiver.join("/"),
            mark(st.receiver_outdated())
        );
    }
    if !flash_requested {
        return Ok(());
    }
    if let Some(reason) = blocker(&st, part, force, dry) {
        println!("{reason}");
        return Ok(());
    }
    if !st.bundle_verified() && !dry && !allow_unverified {
        println!(
            "{} is newer than this version of glorious-linux knows, so its checksum can't be verified.\n\
             Update glorious-linux, or pass --allow-unverified to flash it anyway.",
            bundle_name(&st.url)
        );
        return Ok(());
    }
    if !dry {
        println!("Do not unplug anything while flashing. Type 'yes' to start:");
        let mut answer = String::new();
        std::io::stdin().lock().read_line(&mut answer)?;
        if answer.trim() != "yes" {
            return Ok(());
        }
    }
    let bar = std::cell::Cell::new(false);
    flash(&st, part, force, dry, allow_unverified, &|event| match event {
        FlashEvent::Log(line) => {
            if bar.replace(false) {
                eprintln!();
            }
            println!("{line}");
        }
        FlashEvent::Step(n, of, title) => println!("\n[{n}/{of}] {title}"),
        FlashEvent::Progress(p) => {
            let filled = (p * 30.0).round() as usize;
            eprint!("\r  [{}{}] {:>3.0}%", "#".repeat(filled), " ".repeat(30 - filled.min(30)), p * 100.0);
            bar.set(true);
        }
    })?;
    if bar.get() {
        eprintln!();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{dfu_util_percent, fetch_bundle, known_hash};

    const D3_BLACK: &str =
        "https://nyc3.digitaloceanspaces.com/gloriouscore/Glorious_Core/Model_D3_Wireless_Black-1.3.3_1.1.0-1.3.0_1.0.0.zip";

    #[test]
    fn pins_known_bundles_only() {
        assert!(known_hash(D3_BLACK).is_some());
        assert!(known_hash("https://example.com/Model_D3_Wireless_Black-9.9.9.zip").is_none());
    }

    /// Downloads the real bundle: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn downloads_and_verifies_a_known_bundle() {
        let files = fetch_bundle(D3_BLACK, false, &|_| {}).unwrap();
        let dir = std::path::Path::new(&files.gd32).parent().unwrap().to_path_buf();
        let mouse = crate::nordic_dfu::Package::load(std::path::Path::new(&files.mouse)).unwrap();
        let nrf = crate::nordic_dfu::Package::load(std::path::Path::new(&files.nrf)).unwrap();
        assert_eq!((mouse.init.len(), mouse.firmware.len()), (141, 164_840));
        assert_eq!((nrf.init.len(), nrf.firmware.len()), (140, 50_188));
        assert_eq!(std::fs::metadata(&files.gd32).unwrap().len(), 28_052);
        let mode = std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&dir).unwrap().permissions());
        assert_eq!(mode & 0o777, 0o700, "temp dir must be private");
        drop(files);
        assert!(!dir.exists(), "temp dir must be removed");
    }

    #[test]
    fn parses_dfu_util_progress_lines() {
        assert_eq!(dfu_util_percent("Download\t[==========               ]  41%        11264 bytes"), Some(0.41));
        assert_eq!(dfu_util_percent("Download\t[=========================] 100%        28052 bytes"), Some(1.0));
        assert_eq!(dfu_util_percent("Erase   \t[====                     ]  16%         4096 bytes"), Some(0.16));
        assert_eq!(dfu_util_percent("Downloading element to address = 0x08000000, size = 28052"), None);
        assert_eq!(dfu_util_percent("Download done."), None);
        assert_eq!(dfu_util_percent("Opening DFU capable USB device..."), None);
    }
}
