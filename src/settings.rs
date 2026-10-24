//! Mouse settings as the GUI edits them.
//!
//! The mouse cannot report DPI stages, polling rate, lift-off, debounce, motion sync or button
//! bindings back, so - like Glorious CORE - the last applied values are kept on disk.
use crate::v3::{lookup, parse_key, Mouse, BUTTONS, EFFECTS, FUNCTIONS};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DpiStage {
    pub dpi: u16,
    pub color: [u8; 3],
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Performance {
    pub stages: Vec<DpiStage>,
    /// Index into `stages`.
    pub active: usize,
    pub polling_wireless: u16,
    pub polling_wired: u16,
    /// 0 = 0.7 mm, 1 = 1 mm, 2 = 2 mm
    pub lift_off: u8,
    pub motion_sync: bool,
    pub debounce_mechanical: u8,
    pub debounce_optical: u8,
    pub sleep_minutes: u32,
}

impl Default for Performance {
    /// CORE's Model D3 defaults (DPIStageDefaults_MouseV3 and defaultProfileData).
    fn default() -> Self {
        let stage = |dpi, color, enabled| DpiStage { dpi, color, enabled };
        Self {
            stages: vec![
                stage(400, [0x64, 0x19, 0x00], true),
                stage(800, [0x00, 0x00, 0xFF], true),
                stage(1600, [0xFF, 0x00, 0x00], true),
                stage(3200, [0x00, 0xFF, 0x00], true),
                stage(6400, [0x55, 0x00, 0xFF], false),
                stage(12800, [0x00, 0xFF, 0xFF], false),
            ],
            active: 2,
            polling_wireless: 1000,
            polling_wired: 1000,
            lift_off: 0,
            motion_sync: true,
            debounce_mechanical: 2,
            debounce_optical: 0,
            sleep_minutes: 7,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LightingSettings {
    pub effect: String,
    pub brightness: u8,
    pub wireless_brightness: u8,
    pub speed: u16,
    pub custom_color: bool,
    pub color: [u8; 3],
}

impl Default for LightingSettings {
    fn default() -> Self {
        Self { effect: "rainbow".into(), brightness: 60, wireless_brightness: 60, speed: 50, custom_color: false, color: [255, 136, 0] }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub performance: Performance,
    pub lighting: LightingSettings,
    /// (button, action): action is a mouse function name or `key:<name>`.
    pub buttons: Vec<(String, String)>,
}

impl Default for Settings {
    fn default() -> Self {
        let buttons = [("left", "left"), ("right", "right"), ("middle", "middle"), ("forward", "forward"), ("back", "back"), ("dpi", "dpi-up")];
        Self {
            performance: Performance::default(),
            lighting: LightingSettings::default(),
            buttons: buttons.iter().map(|(b, a)| (b.to_string(), a.to_string())).collect(),
        }
    }
}

fn path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
    base.join("glorious-linux").join("mouse.json")
}

impl Settings {
    /// `None` when nothing has been saved yet.
    pub fn load() -> Option<Settings> {
        serde_json::from_str(&fs::read_to_string(path()).ok()?).ok()
    }

    pub fn save(&self) -> Result<()> {
        let p = path();
        fs::create_dir_all(p.parent().unwrap())?;
        fs::write(&p, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

/// Same order as CORE's ModelV3DeviceHandler.setPerformance; polling rates are only sent when
/// they changed, because the mouse briefly reconnects after a polling change.
pub fn apply_performance(m: &Mouse, p: &Performance, previous: &Performance) -> Result<()> {
    let slots: Vec<Option<(u16, [u8; 3])>> = p.stages.iter().map(|s| s.enabled.then_some((s.dpi, s.color))).collect();
    if slots.get(p.active).copied().flatten().is_none() {
        return Err(anyhow!("the active DPI stage must be enabled"));
    }
    m.set_dpi_slots(&slots, p.active as u8)?;
    m.set_lift_off(p.lift_off)?;
    m.set_motion_sync(p.motion_sync)?;
    m.set_debounce(Some(p.debounce_mechanical), Some(p.debounce_optical))?;
    m.set_power_saving(p.sleep_minutes)?;
    if p.polling_wireless != previous.polling_wireless {
        m.set_polling(p.polling_wireless, true)?;
    }
    if p.polling_wired != previous.polling_wired {
        m.set_polling(p.polling_wired, false)?;
    }
    Ok(())
}

pub fn apply_lighting(m: &Mouse, l: &LightingSettings) -> Result<()> {
    let effect = lookup(&EFFECTS, &l.effect).ok_or_else(|| anyhow!("unknown effect {}", l.effect))?;
    m.set_lighting(effect, l.brightness, l.wireless_brightness, l.speed, l.custom_color.then_some(l.color))
}

pub fn apply_buttons(m: &Mouse, buttons: &[(String, String)]) -> Result<()> {
    for (button, action) in buttons {
        apply_button(m, button, action)?;
    }
    Ok(())
}

pub fn apply_button(m: &Mouse, button: &str, action: &str) -> Result<()> {
    let id = lookup(&BUTTONS, button).ok_or_else(|| anyhow!("unknown button {button}"))?;
    if let Some(key) = action.strip_prefix("key:") {
        return m.bind(id, None, Some(parse_key(key)?));
    }
    let &(_, t, a1, a2) = FUNCTIONS.iter().find(|f| f.0 == action).ok_or_else(|| anyhow!("unknown action {action}"))?;
    m.bind(id, Some((t, a1, a2)), None)
}
