//! GMMK 3 HE status. Device I/O goes through inglorious-core-linux (`glorious_core_rs`), which
//! implements the keyboard protocol ("Magneton" in Glorious CORE 2.1.23); this module only
//! compares the firmware it reports with Glorious' release manifest.
use crate::firmware;
use crate::kbdevice as device;
use anyhow::{anyhow, Result};

/// The configurator library supports the 75% ANSI board.
pub const NAME: &str = "GMMK 3 HE 75% ANSI";
const PRODUCT: &str = "GMMK3HE75WiredANSI";

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

/// `fw` as the configurator reports it: "KBv103 / RFv200 / LEDv103".
pub fn compare(fw: &str) -> Result<Vec<Component>> {
    let manifest = firmware::manifest()?;
    let entry = manifest["Keyboard"].as_array().and_then(|a| a.iter().find(|e| e["productId"] == PRODUCT));
    let mut out = Vec::new();
    for (name, prefix, key) in
        [("device", "KBv", "version_device"), ("rf", "RFv", "version_rf"), ("led", "LEDv", "version_led")]
    {
        let installed = fw
            .split(" / ")
            .find_map(|part| part.strip_prefix(prefix))
            .and_then(|v| u16::from_str_radix(v, 16).ok())
            .ok_or_else(|| anyhow!("unexpected firmware string {fw:?}"))?;
        let latest = entry.and_then(|e| e[key].as_str()).unwrap_or("?").to_string();
        out.push(Component { name, installed, latest });
    }
    Ok(out)
}

pub fn cli() -> Result<()> {
    let transport = device::Transport::open()?;
    let device::Event::Info { fw, profile, layer, sleep, .. } = device::info(&transport)? else {
        return Err(anyhow!("unexpected reply from the keyboard"));
    };
    println!("Keyboard: {NAME}");
    println!("Active:   profile {}, {} layer", profile + 1, ["Base", "Fn", "Fn2"][layer as usize % 3]);
    println!("Sleep:    {}", sleep.map_or("never".into(), |s| format!("after {} min", s / 60)));
    for c in compare(&fw)? {
        let mark = if c.outdated() { "  <- update available" } else { "" };
        println!("  {:<7}: {:04x}   latest {}{mark}", c.name, c.installed, c.latest_display());
    }
    Ok(())
}
