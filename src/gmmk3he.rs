//! Read-only status for the GMMK 3 HE ("Magneton" in Glorious CORE 2.1.23).
//!
//! 64-byte feature reports without a report ID on the 0xFFFF/0x02 interface; byte 7 is
//! `255 - (sum of bytes 0..6)`. Only query commands are sent here; configuration is left to
//! inglorious-core, which implements the write side of the same protocol.
use crate::{firmware, hid};
use anyhow::{anyhow, bail, Result};
use hidapi::HidApi;
use std::{thread::sleep, time::Duration};

const VID: u16 = 0x342D;
/// (PID, CORE product id, display name)
const MODELS: [(u16, &str, &str); 3] = [
    (0xE3DD, "GMMK3HE65WiredANSI", "GMMK 3 HE 65% ANSI"),
    (0xE3DE, "GMMK3HE75WiredANSI", "GMMK 3 HE 75% ANSI"),
    (0xE3DF, "GMMK3HE100WiredANSI", "GMMK 3 HE 100% ANSI"),
];
const GET_KB_FW: u8 = 0xF0;
const GET_RF_FW: u8 = 0x80;
const GET_LED_FW: u8 = 0xAE;
const GET_PROFILE: u8 = 0x85;

#[derive(Clone, Debug)]
pub struct Component {
    pub name: &'static str,
    pub installed: u16,
    /// Manifest versions are hex strings without a fixed width, e.g. "0103" or "200".
    pub latest: String,
}

impl Component {
    /// The manifest version padded like the installed one, e.g. "200" -> "0200".
    pub fn latest_display(&self) -> String {
        u16::from_str_radix(&self.latest, 16).map_or(self.latest.clone(), |v| format!("{v:04x}"))
    }

    pub fn outdated(&self) -> bool {
        u16::from_str_radix(&self.latest, 16).is_ok_and(|l| self.installed < l)
    }
}

#[derive(Clone, Debug)]
pub struct Info {
    pub name: &'static str,
    pub profile: u8,
    pub layer: u8,
    pub components: Vec<Component>,
}

fn query(dev: &hidapi::HidDevice, command: u8) -> Result<Vec<u8>> {
    let mut report = [0u8; 65]; // [report id 0, 64 command bytes]
    report[1] = command;
    report[8] = 255 - (report[1..8].iter().fold(0u8, |a, b| a.wrapping_add(*b)));
    hid::send_feature(dev, &report)?;
    sleep(Duration::from_millis(150));
    let reply = hid::get_feature(dev, 0, 65)?;
    if reply[1] != command {
        bail!("unexpected reply to 0x{command:02x}");
    }
    Ok(reply[1..].to_vec())
}

pub fn read() -> Result<Info> {
    let api = HidApi::new()?;
    let pids: Vec<u16> = MODELS.iter().map(|m| m.0).collect();
    let kb = hid::open(&api, VID, &pids, 0xFFFF, 0x02)?.ok_or_else(|| anyhow!("no GMMK 3 HE found"))?;
    let (_, product, name) = *MODELS.iter().find(|m| m.0 == kb.pid).ok_or_else(|| anyhow!("unknown keyboard"))?;
    let mut installed = Vec::new();
    for (name, cmd) in [("device", GET_KB_FW), ("rf", GET_RF_FW), ("led", GET_LED_FW)] {
        let r = query(&kb.dev, cmd)?;
        installed.push((name, u16::from_le_bytes([r[1], r[2]])));
    }
    let profile = query(&kb.dev, GET_PROFILE)?;
    drop(kb);

    let manifest = firmware::manifest()?;
    let entry = manifest["Keyboard"].as_array().and_then(|a| a.iter().find(|e| e["productId"] == product));
    let components = installed
        .into_iter()
        .map(|(name, have)| Component {
            name,
            installed: have,
            latest: entry.and_then(|e| e[format!("version_{name}")].as_str()).unwrap_or("?").to_string(),
        })
        .collect();
    Ok(Info { name, profile: profile[1], layer: profile[2], components })
}

pub fn cli() -> Result<()> {
    let info = read()?;
    println!("Keyboard: {}", info.name);
    println!("Active:   profile {}, layer {}", info.profile + 1, info.layer + 1);
    for c in &info.components {
        let mark = if c.outdated() { "  <- update available" } else { "" };
        println!("  {:<7}: {:04x}   latest {}{mark}", c.name, c.installed, c.latest_display());
    }
    Ok(())
}
