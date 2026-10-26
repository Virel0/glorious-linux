//! Display text and colours derived from what the mouse reports.
use crate::icons as icon;
use crate::theme::{ERR, OK, WARN};
use crate::v3::EFFECTS;
use eframe::egui::Color32;

/// "dpi-up" -> "DPI up"
pub(super) fn pretty(name: &str) -> String {
    let words: Vec<String> = name.split('-').map(|w| if w == "dpi" { "DPI".into() } else { w.to_string() }).collect();
    let s = words.join(" ");
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// "MOUSE_D3_WIRELESS" -> "Model D3 Wireless"
pub(super) fn model_display(raw: &str) -> String {
    let words: Vec<String> =
        raw.split('_').skip(1).map(|w| if w.len() == 2 { w.to_string() } else { pretty(&w.to_lowercase()) }).collect();
    if words.is_empty() {
        raw.to_string()
    } else {
        format!("Model {}", words.join(" "))
    }
}

/// "ModelD3Wireless" -> "Model D3 Wireless"
pub(super) fn product_display(id: &str) -> String {
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for c in id.chars() {
        if let Some(p) = prev {
            if (c.is_uppercase() && !p.is_uppercase()) || (c.is_ascii_digit() && p.is_lowercase()) {
                out.push(' ');
            }
        }
        out.push(c);
        prev = Some(c);
    }
    out
}

pub(super) fn effect_name(id: u8) -> String {
    EFFECTS.iter().find(|e| e.1 == id).map_or(format!("Effect {id}"), |e| pretty(e.0))
}

/// Why the mouse is not answering, from the error text.
#[derive(Debug, PartialEq)]
pub(super) enum MouseState {
    Ok,
    Asleep,
    Missing,
    NoPermission,
    Other,
}

pub(super) fn mouse_state(error: &Option<String>) -> MouseState {
    match error.as_deref() {
        None => MouseState::Ok,
        Some(e) if e.contains("asleep") => MouseState::Asleep,
        Some(e) if e.contains("no Glorious") => MouseState::Missing,
        Some(e) if e.contains("cannot open") => MouseState::NoPermission,
        Some(_) => MouseState::Other,
    }
}

pub(super) fn battery_icon(level: u8, charging: bool) -> &'static str {
    match (charging, level) {
        (true, _) => icon::BATTERY_CHARGING,
        (_, 0..=15) => icon::BATTERY_EMPTY,
        (_, 16..=40) => icon::BATTERY_LOW,
        (_, 41..=70) => icon::BATTERY_MEDIUM,
        (_, 71..=90) => icon::BATTERY_HIGH,
        _ => icon::BATTERY_FULL,
    }
}

pub(super) fn battery_color(level: u8) -> Color32 {
    match level {
        0..=15 => ERR,
        16..=30 => WARN,
        _ => OK,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_read_like_core() {
        assert_eq!(pretty("dpi-up"), "DPI up");
        assert_eq!(pretty("seamless-breathing"), "Seamless breathing");
        assert_eq!(pretty(""), "");
        assert_eq!(model_display("MOUSE_D3_WIRELESS"), "Model D3 Wireless");
        assert_eq!(model_display("UNKNOWN"), "UNKNOWN");
        assert_eq!(product_display("ModelD3Wireless"), "Model D3 Wireless");
        assert_eq!(effect_name(11), "Ripple");
        assert_eq!(effect_name(99), "Effect 99");
    }

    #[test]
    fn mouse_state_follows_the_error_text() {
        let state = |e: &str| mouse_state(&Some(e.to_string()));
        assert_eq!(mouse_state(&None), MouseState::Ok);
        assert_eq!(state("no reply from the mouse - it is probably asleep"), MouseState::Asleep);
        assert_eq!(state("no Glorious Model O3/D3 or V3 receiver found"), MouseState::Missing);
        assert_eq!(state("cannot open /dev/hidraw3"), MouseState::NoPermission);
        assert_eq!(state("something else"), MouseState::Other);
    }

    #[test]
    fn battery_icon_and_colour_by_level() {
        assert_eq!(battery_icon(5, true), icon::BATTERY_CHARGING);
        assert_eq!(battery_icon(15, false), icon::BATTERY_EMPTY);
        assert_eq!(battery_icon(91, false), icon::BATTERY_FULL);
        assert_eq!(battery_color(15), ERR);
        assert_eq!(battery_color(30), WARN);
        assert_eq!(battery_color(31), OK);
    }
}
