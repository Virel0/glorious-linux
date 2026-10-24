//! Model O3 / D3 protocol, re-implemented from Glorious CORE 2.1.23 (ModelV3DeviceHandler).
//!
//! Every command is a 64-byte feature report with report ID 3: `[0x03, target, cmd, len, payload..]`.
//! Target 0xBB is the mouse (the receiver relays it over 2.4 GHz), 0xAA the receiver itself.
//! Reads are a SET_FEATURE followed ~100 ms later by a GET_FEATURE whose reply echoes the
//! 4-byte header, for both the wired and the receiver path.
use crate::hid;
use anyhow::{bail, Result};
use hidapi::HidApi;
use std::{thread::sleep, time::Duration};

pub const VID: u16 = 0x3794;
pub const REPORT_ID: u8 = 0x03;
pub const MOUSE: u8 = 0xBB;
pub const RECEIVER: u8 = 0xAA;
pub const REPORT_LEN: usize = 64;
pub const PID_RECEIVER: u16 = 0xA300;
pub const PID_RECEIVER_BOOT: u16 = 0xA301;
/// (raw model name, CORE product id, wired PID, wired bootloader PID)
pub const MODELS: [(&str, &str, u16, u16); 2] = [
    ("MOUSE_D3_WIRELESS", "ModelD3Wireless", 0xA322, 0xA323),
    ("MOUSE_O3_WIRELESS", "ModelO3Wireless", 0xA310, 0xA311),
];

const CMD_POWER_SAVING: u8 = 0x04;
const CMD_DEBOUNCE_MECH: u8 = 0x06;
const CMD_DEBOUNCE_OPTICAL: u8 = 0x07;
const CMD_LIFT_OFF: u8 = 0x09;
const CMD_POLL_WIRED: u8 = 0x0D;
const CMD_POLL_WIRELESS: u8 = 0x0E;
const CMD_DPI_STAGE: u8 = 0x0F;
const CMD_DPI_ACTIVE: u8 = 0x10;
const CMD_LIGHTING: u8 = 0x14;
const CMD_MOTION_SYNC: u8 = 0x15;
const CMD_BUTTON: u8 = 0x18;
const CMD_GET_FIRMWARE: u8 = 0x80;
const CMD_GET_SERIAL: u8 = 0x81;
const CMD_GET_BATTERY: u8 = 0x82;
const CMD_GET_POWER_SAVING: u8 = 0x84;
const CMD_GET_LIGHTING: u8 = 0x94;
const CMD_GET_DEVICE_INFO: u8 = 0x99;
const RCMD_FIRMWARE: u8 = 0x80;
const RCMD_FIRMWARE_2: u8 = 0xEE;
const LOW_POWER_MS: u32 = 60_000;
const GET: u8 = 0x80;

#[rustfmt::skip]
pub const EFFECTS: [(&str, u8); 12] = [
    ("off", 0), ("rainbow", 1), ("breathing-single", 2), ("breathing", 3), ("seamless-breathing", 4),
    ("solid", 5), ("fairies", 6), ("ghost", 7), ("cascade", 8), ("prismatic", 9), ("wave", 10), ("ripple", 11),
];
pub const BUTTONS: [(&str, u8); 6] =
    [("left", 0), ("right", 1), ("middle", 2), ("forward", 7), ("back", 8), ("dpi", 19)];
/// (name, function type, arg1, arg2) from CORE's MouseFunctionToHIDMap.
#[rustfmt::skip]
pub const FUNCTIONS: [(&str, u8, u8, u8); 13] = [
    ("left", 1, 0, 0), ("right", 1, 1, 0), ("middle", 1, 2, 0), ("back", 1, 3, 0), ("forward", 1, 4, 0),
    ("dpi-up", 8, 9, 0), ("dpi-down", 8, 10, 0), ("dpi-shift", 8, 14, 0), ("battery", 8, 6, 0),
    ("scroll-up", 2, 0, 0), ("scroll-down", 2, 1, 0), ("profile-cycle", 8, 2, 0), ("disable", 0, 0, 0),
];
pub const FN_KEYBOARD: u8 = 4;
#[rustfmt::skip]
pub const DEFAULT_STAGE_COLORS: [[u8; 3]; 6] =
    [[0x64, 0x19, 0x00], [0, 0, 0xFF], [0xFF, 0, 0], [0, 0xFF, 0], [0x55, 0, 0xFF], [0, 0xFF, 0xFF]];

pub fn lookup<T: Copy>(table: &[(&str, T)], name: &str) -> Option<T> {
    table.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
}

/// Bindable keyboard keys as (name, HID usage).
pub fn key_names() -> Vec<(String, u8)> {
    let mut keys: Vec<(String, u8)> = (b'a'..=b'z').map(|c| ((c as char).to_string(), c - b'a' + 0x04)).collect();
    keys.extend((1..=9).map(|n| (n.to_string(), 0x1D + n)));
    keys.push(("0".into(), 0x27));
    keys.extend((1..=12).map(|n| (format!("f{n}"), 0x39 + n)));
    for (name, code) in [
        ("enter", 0x28),
        ("esc", 0x29),
        ("backspace", 0x2A),
        ("tab", 0x2B),
        ("space", 0x2C),
        ("insert", 0x49),
        ("home", 0x4A),
        ("pageup", 0x4B),
        ("delete", 0x4C),
        ("end", 0x4D),
        ("pagedown", 0x4E),
        ("right", 0x4F),
        ("left", 0x50),
        ("down", 0x51),
        ("up", 0x52),
        ("ctrl", 0xE0),
        ("shift", 0xE1),
        ("alt", 0xE2),
        ("super", 0xE3),
    ] {
        keys.push((name.into(), code));
    }
    keys
}

/// A key name from `key_names()` or a hex HID usage such as `0x04`.
pub fn parse_key(s: &str) -> Result<u8> {
    let s = s.to_ascii_lowercase();
    if let Some((_, code)) = key_names().into_iter().find(|(n, _)| *n == s) {
        return Ok(code);
    }
    u8::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|_| anyhow::anyhow!("unknown key {s:?}"))
}

/// Everything the mouse can report about itself.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub via_receiver: bool,
    pub model: String,
    pub color: String,
    pub firmware: String,
    pub receiver_firmware: Option<[String; 2]>,
    pub battery: Battery,
    pub lighting: Lighting,
    pub sleep_minutes: u32,
}

fn battery_status(code: u8) -> &'static str {
    match code {
        1 => "charging",
        2 => "discharging",
        3 => "charged",
        _ => "missing",
    }
}

#[derive(Clone, Debug)]
pub struct Battery {
    pub level: u8,
    pub status: &'static str,
    pub millivolts: u16,
    pub secondary_level: u8,
    pub secondary_status: &'static str,
}

#[derive(Clone, Debug)]
pub struct Lighting {
    pub effect: u8,
    pub wired_brightness: u8,
    pub wireless_brightness: u8,
    pub speed: u16,
    /// `Some(rgb)` in single-colour mode, `None` for the effect's multicolour palette.
    pub color: Option<[u8; 3]>,
}

/// Serialises access: two threads talking to the receiver at once garble each other's replies.
static HID_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub struct Mouse {
    _lock: std::sync::MutexGuard<'static, ()>,
    pub dev: hidapi::HidDevice,
    pub path: String,
    pub pid: u16,
    pub verbose: bool,
}

impl Mouse {
    /// Prefer a cable connection, then the receiver.
    pub fn open(api: &HidApi, verbose: bool) -> Result<Mouse> {
        let pids: Vec<u16> = MODELS.iter().map(|m| m.2).chain([PID_RECEIVER]).collect();
        Self::open_pids(api, &pids, verbose)
    }

    /// Open one specific device, e.g. the receiver while the mouse is on its cable.
    pub fn open_pids(api: &HidApi, pids: &[u16], verbose: bool) -> Result<Mouse> {
        let lock = HID_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        match hid::open(api, VID, pids, 0xFF00, 0x01)? {
            Some(o) => Ok(Mouse { _lock: lock, dev: o.dev, path: o.path, pid: o.pid, verbose }),
            None => bail!("no Glorious Model O3/D3 or V3 receiver found"),
        }
    }

    pub fn via_receiver(&self) -> bool {
        self.pid == PID_RECEIVER
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        Ok(Snapshot {
            via_receiver: self.via_receiver(),
            model: self.model_name()?,
            color: self.color()?,
            firmware: self.firmware()?,
            receiver_firmware: if self.via_receiver() { Some(self.receiver_firmware()?) } else { None },
            battery: self.battery()?,
            lighting: self.lighting()?,
            sleep_minutes: self.power_saving()?.0 / 60_000,
        })
    }

    fn log(&self, tag: &str, buf: &[u8]) {
        if self.verbose {
            let hex: Vec<String> = buf[..24].iter().map(|b| format!("{b:02x}")).collect();
            eprintln!("  {tag} {}", hex.join(" "));
        }
    }

    pub fn send(&self, target: u8, cmd: u8, payload: &[u8], delay_ms: u64) -> Result<()> {
        let mut buf = [0u8; REPORT_LEN];
        buf[..4].copy_from_slice(&[REPORT_ID, target, cmd, payload.len() as u8]);
        buf[4..4 + payload.len()].copy_from_slice(payload);
        self.log(">>", &buf);
        hid::send_feature(&self.dev, &buf)?;
        sleep(Duration::from_millis(delay_ms));
        Ok(())
    }

    /// Send a command and return the reply payload.
    pub fn query(&self, target: u8, cmd: u8, payload: &[u8]) -> Result<Vec<u8>> {
        self.query_after(target, cmd, payload, 100)
    }

    /// `query` with a custom wait between the request and reading the reply.
    pub fn query_after(&self, target: u8, cmd: u8, payload: &[u8], wait_ms: u64) -> Result<Vec<u8>> {
        self.send(target, cmd, payload, wait_ms)?;
        // A sleeping mouse makes the receiver return an empty report.
        let asleep = || anyhow::anyhow!("no reply from the mouse - it is probably asleep; move it and try again");
        let buf = hid::get_feature(&self.dev, REPORT_ID, REPORT_LEN).map_err(|_| asleep())?;
        self.log("<<", &buf);
        if buf[2] != cmd {
            return Err(asleep());
        }
        let len = (buf[3] as usize).min(REPORT_LEN - 4);
        let mut payload = buf[4..4 + len].to_vec();
        payload.resize(REPORT_LEN - 4, 0);
        Ok(payload)
    }

    pub fn model_name(&self) -> Result<String> {
        let v = self.query(MOUSE, CMD_GET_DEVICE_INFO, &[1])?;
        let end = v.iter().position(|&b| b == 0).unwrap_or(v.len()).min(30);
        Ok(String::from_utf8_lossy(&v[..end]).trim().to_string())
    }

    pub fn color(&self) -> Result<String> {
        Ok(match self.query(MOUSE, CMD_GET_SERIAL, &[])?[0] {
            0x42 => "Black".into(),
            0x57 => "White".into(),
            c => format!("unknown (0x{c:02x})"),
        })
    }

    pub fn firmware(&self) -> Result<String> {
        let v = self.query(MOUSE, CMD_GET_FIRMWARE, &[])?;
        Ok(format!("{}.{}.{}", v[0], v[1], v[2]))
    }

    /// The receiver reports two firmware versions (its nRF52 and GD32 chips).
    pub fn receiver_firmware(&self) -> Result<[String; 2]> {
        let a = self.query(RECEIVER, RCMD_FIRMWARE, &[])?;
        let b = self.query(RECEIVER, RCMD_FIRMWARE_2, &[])?;
        Ok([format!("{}.{}.{}", a[0], a[1], a[2]), format!("{}.{}.{}", b[0], b[1], b[2])])
    }

    /// Two `[status, level %, millivolts (BE16)]` blocks; CORE shows the second as the battery.
    pub fn battery(&self) -> Result<Battery> {
        let v = self.query(MOUSE, CMD_GET_BATTERY, &[])?;
        Ok(Battery {
            level: v[5],
            status: battery_status(v[4]),
            millivolts: u16::from_be_bytes([v[6], v[7]]),
            secondary_level: v[1],
            secondary_status: battery_status(v[0]),
        })
    }

    pub fn lighting(&self) -> Result<Lighting> {
        let v = self.query(MOUSE, CMD_GET_LIGHTING, &[])?;
        Ok(Lighting {
            effect: v[0],
            wired_brightness: v[1],
            wireless_brightness: v[2],
            speed: u16::from_be_bytes([v[3], v[4]]),
            color: (v[6] == 1).then_some([v[7], v[8], v[9]]),
        })
    }

    // Reads: every setter has a matching getter at `opcode | 0x80` (found by probing a D3;
    // CORE itself only uses the battery, firmware, lighting and power-saving ones).

    /// Index of the DPI stage the mouse is using right now.
    pub fn active_stage(&self) -> Result<u8> {
        Ok(self.query(MOUSE, CMD_DPI_ACTIVE | GET, &[])?[0])
    }

    /// Like `active_stage`, but quick enough to poll a few times a second.
    pub fn active_stage_fast(&self) -> Result<u8> {
        Ok(self.query_after(MOUSE, CMD_DPI_ACTIVE | GET, &[], 25)?[0])
    }

    /// `None` when the slot is switched off.
    pub fn dpi_stage(&self, slot: u8) -> Result<Option<(u16, [u8; 3])>> {
        let v = self.query(MOUSE, CMD_DPI_STAGE | GET, &[slot])?;
        let dpi = u16::from_be_bytes([v[1], v[2]]);
        Ok((dpi != 0).then_some((dpi, [v[3], v[4], v[5]])))
    }

    pub fn polling(&self, wireless: bool) -> Result<u16> {
        let v = self.query(MOUSE, if wireless { CMD_POLL_WIRELESS } else { CMD_POLL_WIRED } | GET, &[])?;
        Ok(u16::from_be_bytes([v[0], v[1]]))
    }

    pub fn lift_off(&self) -> Result<u8> {
        Ok(self.query(MOUSE, CMD_LIFT_OFF | GET, &[])?[0])
    }

    /// (mechanical ms, optical ms)
    pub fn debounce(&self) -> Result<(u8, u8)> {
        let mech = self.query(MOUSE, CMD_DEBOUNCE_MECH | GET, &[])?[0];
        let optical = self.query(MOUSE, CMD_DEBOUNCE_OPTICAL | GET, &[])?[0];
        Ok((mech, optical))
    }

    pub fn motion_sync(&self) -> Result<bool> {
        Ok(self.query(MOUSE, CMD_MOTION_SYNC | GET, &[])?[0] != 0)
    }

    /// (function type, arg1, arg2) as written by `bind`.
    pub fn binding(&self, button: u8) -> Result<(u8, u8, u8)> {
        let v = self.query(MOUSE, CMD_BUTTON | GET, &[button])?;
        Ok((v[1], v[3], v[5]))
    }

    /// (sleep ms, low-power ms)
    pub fn power_saving(&self) -> Result<(u32, u32)> {
        let v = self.query(MOUSE, CMD_GET_POWER_SAVING, &[])?;
        Ok((u32::from_be_bytes([v[0], v[1], v[2], v[3]]), u32::from_be_bytes([v[4], v[5], v[6], v[7]])))
    }

    /// Six DPI slots, `None` = disabled (CORE writes an all-zero stage for those).
    /// CORE selects the active stage before writing the stages, so this does too.
    pub fn set_dpi_slots(&self, slots: &[Option<(u16, [u8; 3])>], active: u8) -> Result<()> {
        self.send(MOUSE, CMD_DPI_ACTIVE, &[active.min(5)], 30)?;
        for i in 0..6u8 {
            let mut p = [0u8; 9];
            p[0] = i;
            if let Some(&Some((dpi, rgb))) = slots.get(i as usize) {
                p[1..3].copy_from_slice(&dpi.clamp(50, 26000).to_be_bytes());
                p[3..6].copy_from_slice(&rgb);
                p[6..9].copy_from_slice(&rgb);
            }
            self.send(MOUSE, CMD_DPI_STAGE, &p, 30)?;
        }
        Ok(())
    }

    /// Up to six consecutive stages; the remaining slots are disabled.
    pub fn set_dpi_stages(&self, stages: &[u16], colors: &[[u8; 3]], active: u8) -> Result<()> {
        let slots: Vec<_> = stages.iter().zip(colors).map(|(&d, &c)| Some((d, c))).collect();
        self.set_dpi_slots(&slots, active)
    }

    pub fn set_polling(&self, hz: u16, wireless: bool) -> Result<()> {
        self.send(MOUSE, if wireless { CMD_POLL_WIRELESS } else { CMD_POLL_WIRED }, &hz.to_be_bytes(), 200)
    }

    /// 0 = 0.7 mm, 1 = 1 mm, 2 = 2 mm
    pub fn set_lift_off(&self, level: u8) -> Result<()> {
        self.send(MOUSE, CMD_LIFT_OFF, &[level.min(2)], 150)
    }

    pub fn set_debounce(&self, mechanical: Option<u8>, optical: Option<u8>) -> Result<()> {
        if let Some(ms) = mechanical {
            self.send(MOUSE, CMD_DEBOUNCE_MECH, &[ms], 150)?;
        }
        if let Some(ms) = optical {
            self.send(MOUSE, CMD_DEBOUNCE_OPTICAL, &[ms, 10, 20], 150)?;
        }
        Ok(())
    }

    pub fn set_motion_sync(&self, on: bool) -> Result<()> {
        self.send(MOUSE, CMD_MOTION_SYNC, &[on as u8], 150)
    }

    pub fn set_power_saving(&self, minutes: u32) -> Result<()> {
        let mut p = [0u8; 8];
        p[..4].copy_from_slice(&(minutes * 60_000).to_be_bytes());
        p[4..].copy_from_slice(&LOW_POWER_MS.to_be_bytes());
        self.send(MOUSE, CMD_POWER_SAVING, &p, 30)
    }

    /// `rgb: None` selects the effect's multicolour mode.
    pub fn set_lighting(
        &self,
        effect: u8,
        brightness: u8,
        wireless_brightness: u8,
        speed: u16,
        rgb: Option<[u8; 3]>,
    ) -> Result<()> {
        let s = speed.max(10).to_be_bytes();
        let [r, g, b] = rgb.unwrap_or([0, 0, 0]);
        let p = [effect, brightness, wireless_brightness, s[0], s[1], 8, rgb.is_some() as u8, r, g, b];
        self.send(MOUSE, CMD_LIGHTING, &p, 30)
    }

    /// `key: Some(usage)` binds a keyboard key (HID usage), otherwise a mouse function.
    pub fn bind(&self, button: u8, function: Option<(u8, u8, u8)>, key: Option<u8>) -> Result<()> {
        let f = match (function, key) {
            (_, Some(code)) => [FN_KEYBOARD, 0, code, 0, code],
            (Some((t, a1, a2)), None) => [t, 0, a1, 0, a2],
            (None, None) => bail!("nothing to bind"),
        };
        self.send(MOUSE, CMD_BUTTON, &[button, f[0], f[1], f[2], f[3], f[4]], 30)
    }
}
