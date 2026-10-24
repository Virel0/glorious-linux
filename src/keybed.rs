//! Drawing of the GMMK 3 HE 75% ANSI board for the keyboard pages.
//!
//! Key positions adapted from inglorious-core-linux (src/ui/widgets.rs, MIT,
//! Copyright (c) 2026 thomas griffin): (physical key name, label, x, y, width in U) on a grid
//! where 1U = 56 px including the 4 px gap.
use crate::theme::*;
use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, Ui};
use glorious_core_rs::tables::PHYSICAL_KEYS;

const KEYBED: &[(&str, &str, f32, f32, f32)] = &[
    ("Escape", "Esc", 20., 20., 1.), ("F1", "F1", 90., 20., 1.), ("F2", "F2", 146., 20., 1.), ("F3", "F3", 202., 20., 1.),
    ("F4", "F4", 258., 20., 1.), ("F5", "F5", 328., 20., 1.), ("F6", "F6", 384., 20., 1.), ("F7", "F7", 440., 20., 1.),
    ("F8", "F8", 496., 20., 1.), ("F9", "F9", 566., 20., 1.), ("F10", "F10", 622., 20., 1.), ("F11", "F11", 678., 20., 1.),
    ("F12", "F12", 734., 20., 1.),
    ("Backquote", "`", 20., 90., 1.), ("Digit1", "1", 76., 90., 1.), ("Digit2", "2", 132., 90., 1.), ("Digit3", "3", 188., 90., 1.),
    ("Digit4", "4", 244., 90., 1.), ("Digit5", "5", 300., 90., 1.), ("Digit6", "6", 356., 90., 1.), ("Digit7", "7", 412., 90., 1.),
    ("Digit8", "8", 468., 90., 1.), ("Digit9", "9", 524., 90., 1.), ("Digit0", "0", 580., 90., 1.), ("Minus", "-", 636., 90., 1.),
    ("Equal", "=", 692., 90., 1.), ("Backspace", "Backspace", 748., 90., 2.), ("Delete", "Del", 860., 90., 1.),
    ("Tab", "Tab", 20., 146., 1.5), ("KeyQ", "Q", 104., 146., 1.), ("KeyW", "W", 160., 146., 1.), ("KeyE", "E", 216., 146., 1.),
    ("KeyR", "R", 272., 146., 1.), ("KeyT", "T", 328., 146., 1.), ("KeyY", "Y", 384., 146., 1.), ("KeyU", "U", 440., 146., 1.),
    ("KeyI", "I", 496., 146., 1.), ("KeyO", "O", 552., 146., 1.), ("KeyP", "P", 608., 146., 1.), ("BracketLeft", "[", 664., 146., 1.),
    ("BracketRight", "]", 720., 146., 1.), ("Backslash", "\\", 776., 146., 1.5), ("PageUp", "PgUp", 860., 146., 1.),
    ("CapsLock", "Caps", 20., 202., 1.75), ("KeyA", "A", 118., 202., 1.), ("KeyS", "S", 174., 202., 1.), ("KeyD", "D", 230., 202., 1.),
    ("KeyF", "F", 286., 202., 1.), ("KeyG", "G", 342., 202., 1.), ("KeyH", "H", 398., 202., 1.), ("KeyJ", "J", 454., 202., 1.),
    ("KeyK", "K", 510., 202., 1.), ("KeyL", "L", 566., 202., 1.), ("Semicolon", ";", 622., 202., 1.), ("Quote", "'", 678., 202., 1.),
    ("Enter", "Enter", 734., 202., 2.25), ("PageDown", "PgDn", 860., 202., 1.),
    ("ShiftLeft", "Shift", 20., 258., 2.25), ("KeyZ", "Z", 146., 258., 1.), ("KeyX", "X", 202., 258., 1.), ("KeyC", "C", 258., 258., 1.),
    ("KeyV", "V", 314., 258., 1.), ("KeyB", "B", 370., 258., 1.), ("KeyN", "N", 426., 258., 1.), ("KeyM", "M", 482., 258., 1.),
    ("Comma", ",", 538., 258., 1.), ("Period", ".", 594., 258., 1.), ("Slash", "/", 650., 258., 1.), ("ShiftRight", "Shift", 706., 258., 1.75),
    ("ArrowUp", egui_phosphor::regular::ARROW_UP, 804., 258., 1.), ("End", "End", 860., 258., 1.),
    ("ControlLeft", "Ctrl", 20., 314., 1.25), ("MetaLeft", "Super", 90., 314., 1.25), ("AltLeft", "Alt", 160., 314., 1.25),
    ("Space", "", 230., 314., 6.25), ("AltRight", "Alt", 580., 314., 1.25), ("Fn", "Fn", 650., 314., 1.25),
    ("ArrowLeft", egui_phosphor::regular::ARROW_LEFT, 748., 314., 1.), ("ArrowDown", egui_phosphor::regular::ARROW_DOWN, 804., 314., 1.), ("ArrowRight", egui_phosphor::regular::ARROW_RIGHT, 860., 314., 1.),
];
/// Colour index of the logo badge.
pub const BADGE: u8 = 158;
const WIDTH: f32 = 932.0;
const HEIGHT: f32 = 386.0;

fn physical(name: &str) -> u8 {
    PHYSICAL_KEYS.iter().find(|(n, _)| *n == name).map(|(_, i)| *i).expect("KEYBED names are PHYSICAL_KEYS names")
}

/// Physical indices of every key cap.
pub fn keys() -> impl Iterator<Item = u8> {
    KEYBED.iter().map(|(n, ..)| physical(n))
}

pub fn label(idx: u8) -> &'static str {
    match KEYBED.iter().find(|(n, ..)| physical(n) == idx) {
        Some((_, "", ..)) => "Space",
        Some((_, l, ..)) => l,
        None if idx == BADGE => "Logo",
        None => "?",
    }
}

#[derive(Default)]
pub struct Look {
    pub fill: Option<Color32>,
    /// Small second line, e.g. an actuation point or a remapped key.
    pub value: Option<String>,
    pub selected: bool,
    pub highlight: bool,
}

/// Readable text colour on a key fill.
fn ink(fill: Color32) -> Color32 {
    let l = 0.299 * fill.r() as f32 + 0.587 * fill.g() as f32 + 0.114 * fill.b() as f32;
    if l > 140.0 { Color32::from_rgb(0x15, 0x15, 0x18) } else { TEXT }
}

/// Draw the board scaled to the available width; returns the clicked key (or BADGE).
pub fn show(ui: &mut Ui, look: &dyn Fn(u8) -> Look) -> Option<u8> {
    let s = (ui.available_width() / WIDTH).clamp(0.6, 1.25);
    let (outer, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), HEIGHT * s), Sense::hover());
    let origin = egui::pos2(outer.center().x - WIDTH * s / 2.0, outer.top());
    let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(origin + egui::vec2(x, y) * s, egui::vec2(w, h) * s);
    let painter = ui.painter().clone();
    painter.rect_filled(at(4.0, 4.0, WIDTH - 8.0, HEIGHT - 16.0), 18.0 * s, PANEL);
    let label_font = FontId::proportional((12.5 * s).max(10.0));
    let value_font = FontId::monospace((10.0 * s).max(9.0));
    let mut clicked = None;
    let mut cap = |ui: &mut Ui, r: Rect, idx: u8, text: &str| {
        let resp = ui.interact(r, ui.id().with(("key", idx)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp.clicked() {
            clicked = Some(idx);
        }
        let l = look(idx);
        let fill = l.fill.unwrap_or(if l.highlight { SURFACE3 } else { SURFACE2 });
        let stroke = if l.selected {
            Stroke::new(2.0_f32, ACCENT)
        } else if resp.hovered() {
            Stroke::new(1.0_f32, DIM)
        } else {
            Stroke::new(1.0_f32, BORDER)
        };
        painter.rect(r, 8.0 * s, fill, stroke, StrokeKind::Inside);
        let color = l.fill.map_or(TEXT, ink);
        match &l.value {
            Some(v) => {
                painter.text(r.center() - egui::vec2(0.0, 7.0 * s), Align2::CENTER_CENTER, text, label_font.clone(), color);
                painter.text(r.center() + egui::vec2(0.0, 9.0 * s), Align2::CENTER_CENTER, v, value_font.clone(), if l.fill.is_some() { color } else { ACCENT });
            }
            None => {
                painter.text(r.center(), Align2::CENTER_CENTER, text, label_font.clone(), color);
            }
        }
    };
    for &(name, text, x, y, u) in KEYBED {
        cap(ui, at(x, y, u * 56.0 - 4.0, 52.0), physical(name), text);
    }
    cap(ui, at(790.0, 20.0, 52.0, 52.0), BADGE, "G");
    // Rotary knob: drawn for orientation only.
    painter.circle(at(860.0, 20.0, 52.0, 52.0).center(), 25.0 * s, SURFACE3, Stroke::new(2.0_f32, BORDER));
    clicked
}
