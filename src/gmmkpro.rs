//! GMMK Pro lighting on its stock firmware. The protocol comes from MechNoxer/Open-Glorious-Core (MIT):
//! 256-byte feature reports with report ID 7 on the vendor interface (usage page 0xFF01, USB
//! interface 2; interface 0 carries the keystrokes and is never opened). Lighting only - nothing
//! here can touch the keyboard's firmware or keymap.
//!
//! A lighting change is three reports: effect properties, effect parameters (speed, brightness,
//! colour), then "commit" for the active profile/layer. The firmware acks reports sent back to back
//! but applies only the first, so each one gets its own open/close and a short pause.
//!
//! Byte 10/12 of the parameter report is a "use the effect's own colours" flag. With it cleared,
//! most animated effects run in the single colour given (tested on a GMMK Pro ANSI): the animation
//! then shows as moving brightness, which is subtler than the rainbow version. A few, such as wave-2,
//! keep their own palette either way.
use crate::hid;
use anyhow::{bail, Context, Result};
use hidapi::HidApi;
use std::{thread, time::Duration};

pub const NAME: &str = "GMMK Pro";
const VID: u16 = 0x320f;
const PID: u16 = 0x5044;
const USAGE_PAGE: u16 = 0xff01;
const USAGE: u16 = 0x0001;
const REPORT_ID: u8 = 7;
const LEN: usize = 256;
const PAUSE: Duration = Duration::from_millis(100);

/// CLI name and the name Glorious CORE uses; the index is the firmware's effect id.
pub const EFFECTS: [(&str, &str); 19] = [
    ("off", "LED Off"),
    ("glorious", "Glorious Mode"),
    ("wave", "Wave 1"),
    ("wave-2", "Wave 2"),
    ("spiral", "Spiraling Wave"),
    ("acid", "Acid Mode"),
    ("breathing", "Breathing"),
    ("solid", "Normally On"),
    ("ripple", "Ripple Graff"),
    ("trace", "Pass Without Trace"),
    ("fast-trace", "Fast Run Without Trace"),
    ("matrix", "Matrix 2"),
    ("matrix-3", "Matrix 3"),
    ("rainbow", "Rainbow"),
    ("heartbeat", "Heartbeat Sensor"),
    ("digital-times", "Digital Times"),
    ("kamehameha", "Kamehameha"),
    ("pingpong", "Pingpong"),
    ("surmount", "Surmount"),
];
const BREATHING: u8 = 6;
const SOLID: u8 = 7;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lighting {
    pub effect: u8,
    /// `None` lets an animated effect use its own colours; solid and breathing then show white.
    pub color: Option<[u8; 3]>,
    /// 0-100 (the keyboard has 21 levels)
    pub brightness: u8,
    /// 0-100
    pub speed: u8,
}

/// An effect by CLI name, CORE name or id.
pub fn effect_id(name: &str) -> Result<u8> {
    let wanted = name.trim().to_lowercase();
    if let Ok(id) = wanted.parse::<u8>() {
        if usize::from(id) < EFFECTS.len() {
            return Ok(id);
        }
    }
    match EFFECTS.iter().position(|(cli, core)| *cli == wanted || core.to_lowercase() == wanted) {
        Some(i) => Ok(i as u8),
        None => {
            let names: Vec<&str> = EFFECTS.iter().map(|(cli, _)| *cli).collect();
            bail!("unknown effect {name:?}; one of: {}", names.join(", "))
        }
    }
}

fn level(brightness: u8) -> u8 {
    ((u16::from(brightness.min(100)) * 20 + 50) / 100) as u8
}

fn report(head: &[u8]) -> Vec<u8> {
    let mut r = vec![0u8; LEN];
    r[0] = REPORT_ID;
    r[1..=head.len()].copy_from_slice(head);
    r
}

/// The three reports of a lighting change, in sending order.
fn lighting_reports(l: &Lighting, profile: u8, layer: u8) -> [Vec<u8>; 3] {
    let lv = level(l.brightness);
    let props = report(&[7, profile, layer, l.effect, 0, 1, 0, lv, 0, 16]);
    let own_colours = l.color.is_none() && !matches!(l.effect, BREATHING | SOLID);
    let [r, g, b] = l.color.unwrap_or([255, 255, 255]);
    let mut params = report(&[2, l.effect]);
    params[8] = 100 - l.speed.min(100);
    params[9] = lv;
    params[10] = u8::from(own_colours);
    params[12] = u8::from(own_colours);
    params[15..18].copy_from_slice(&[r, g, b]);
    let commit = report(&[1, profile, layer]);
    [props, params, commit]
}

/// Profile and layer from the reply to request 0x81; the firmware has used a few layouts.
fn parse_profile_layer(reply: &[u8]) -> Option<(u8, u8)> {
    let ok = |p: u8, l: u8| (1..=3).contains(&p) && (1..=3).contains(&l);
    [(1, 2), (2, 3), (8, 9)].into_iter().map(|(a, b)| (reply[a], reply[b])).find(|&(p, l)| ok(p, l))
}

struct Keyboard {
    api: HidApi,
    path: String,
}

impl Keyboard {
    fn find() -> Result<Self> {
        let api = HidApi::new()?;
        let Some(opened) = hid::open(&api, VID, &[PID], USAGE_PAGE, USAGE)? else {
            bail!("no {NAME} found (stock firmware, 320f:5044); is it plugged in?");
        };
        Ok(Self { api, path: opened.path })
    }

    /// One report per open/close, then a pause, or the firmware drops it.
    fn send(&self, report: &[u8], reply: bool) -> Result<Option<Vec<u8>>> {
        let opened = hid::open(&self.api, VID, &[PID], USAGE_PAGE, USAGE)?
            .with_context(|| format!("{NAME} disappeared from {}", self.path))?;
        hid::send_feature(&opened.dev, report)?;
        thread::sleep(PAUSE);
        if !reply {
            return Ok(None);
        }
        hid::get_feature(&opened.dev, REPORT_ID, LEN).map(Some)
    }

    fn profile_layer(&self) -> Result<(u8, u8)> {
        let reply = self.send(&report(&[0x81, 1, 1]), true)?.unwrap_or_default();
        Ok(parse_profile_layer(&reply).unwrap_or((1, 1)))
    }
}

/// Apply `l` to the active profile/layer; returns them.
pub fn apply(l: &Lighting) -> Result<(u8, u8)> {
    let kb = Keyboard::find()?;
    let (profile, layer) = kb.profile_layer()?;
    for r in lighting_reports(l, profile, layer) {
        kb.send(&r, false)?;
    }
    Ok((profile, layer))
}

pub fn info() -> Result<()> {
    let kb = Keyboard::find()?;
    let (profile, layer) = kb.profile_layer()?;
    println!("Keyboard: {NAME} (stock firmware) on {}", kb.path);
    println!("Active:   profile {profile}, layer {layer}");
    println!("Lighting can be set but not read back (the firmware doesn't report it).");
    println!("Effects:");
    for (i, (cli, core)) in EFFECTS.iter().enumerate() {
        println!("  {i:>2}  {cli:<14} {core}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_effects_by_name_core_name_or_id() {
        assert_eq!(effect_id("wave").unwrap(), 2);
        assert_eq!(effect_id("Normally On").unwrap(), SOLID);
        assert_eq!(effect_id("18").unwrap(), 18);
        assert!(effect_id("19").is_err());
        assert!(effect_id("disco").is_err());
    }

    #[test]
    fn brightness_maps_to_21_levels() {
        assert_eq!([level(0), level(50), level(70), level(100), level(255)], [0, 10, 14, 20, 20]);
    }

    #[test]
    fn builds_the_three_lighting_reports() {
        let l = Lighting { effect: 2, color: Some([0x12, 0x34, 0x56]), brightness: 70, speed: 30 };
        let [props, params, commit] = lighting_reports(&l, 2, 1);
        assert!([&props, &params, &commit].iter().all(|r| r.len() == LEN && r[0] == REPORT_ID));
        assert_eq!(&props[..11], &[7, 7, 2, 1, 2, 0, 1, 0, 14, 0, 16]);
        assert_eq!(&params[..3], &[7, 2, 2]);
        assert_eq!((params[8], params[9], params[10], params[12]), (70, 14, 0, 0));
        assert_eq!(&params[15..18], &[0x12, 0x34, 0x56]);
        assert_eq!(&commit[..4], &[7, 1, 2, 1]);
    }

    #[test]
    fn no_colour_means_own_colours_except_solid_and_breathing() {
        let wave = Lighting { effect: 2, color: None, brightness: 50, speed: 50 };
        assert_eq!(lighting_reports(&wave, 1, 1)[1][10], 1);
        let solid = Lighting { effect: SOLID, ..wave };
        let params = &lighting_reports(&solid, 1, 1)[1];
        assert_eq!((params[10], &params[15..18]), (0, &[255u8, 255, 255][..]));
    }

    #[test]
    fn reads_profile_and_layer_from_any_known_layout() {
        let mut reply = vec![0u8; LEN];
        reply[0] = REPORT_ID;
        reply[1] = 0x81;
        reply[2] = 3;
        reply[3] = 2;
        assert_eq!(parse_profile_layer(&reply), Some((3, 2)));
        assert_eq!(parse_profile_layer(&vec![0u8; LEN]), None);
    }
}
