//! Keymap page: pick a key, then what it sends.
use super::Keyboard;
use crate::icons as icon;
use crate::keybed::{self, Look, BADGE};
use crate::magneton::state::{self, Layer};
use crate::magneton::tables::KEYCODES;
use crate::theme::*;
use eframe::egui::{self, RichText, Ui};
use std::collections::BTreeSet;

/// "KeyA" -> "A", "ArrowLeft" -> "Arrow Left", "mouse_left" -> "Mouse Left".
fn key_display(name: &str) -> String {
    let base = name.strip_prefix("Key").or_else(|| name.strip_prefix("Digit")).unwrap_or(name);
    let mut out = String::new();
    for (i, c) in base.replace('_', " ").chars().enumerate() {
        if i > 0 && c.is_uppercase() && !out.ends_with(' ') {
            out.push(' ');
        }
        let word_start = i == 0 || out.ends_with(' ');
        out.push(if word_start { c.to_ascii_uppercase() } else { c });
    }
    out
}

/// Bindable keycodes, one entry per code.
fn keycodes() -> Vec<(String, u8)> {
    let mut seen = BTreeSet::new();
    KEYCODES.iter().filter(|(_, code)| *code != 0 && seen.insert(*code)).map(|(n, c)| (key_display(n), *c)).collect()
}

fn code_name(code: u8) -> String {
    KEYCODES.iter().find(|(_, c)| *c == code).map_or(format!("0x{code:02x}"), |(n, _)| key_display(n))
}

impl Keyboard {
    pub fn keymap(&mut self, ui: &mut Ui) {
        page_header(ui, "Keymap", "Remap any key on any layer", |_| {});
        self.status_banner(ui);
        self.layer_bar(ui);
        ui.add_space(8.0);
        let selected = self.selected.iter().next().copied();
        let layer = self.draft.current();
        let bindings = layer.bindings.clone();
        let advanced: BTreeSet<u8> = layer.advanced.keys().copied().collect();
        let mut clicked = None;
        card(ui, icon::KEYBOARD, "Keys", |ui| {
            ui.label(RichText::new("Click a key, then pick what it should send.").color(DIM));
            clicked = keybed::show(ui, &|k| Look {
                value: if advanced.contains(&k) {
                    Some("adv".into())
                } else {
                    bindings.get(&k).map(|codes| code_name(codes[0][2]))
                },
                selected: selected == Some(k),
                highlight: bindings.contains_key(&k) || advanced.contains(&k),
                ..Default::default()
            });
        });
        if let Some(k) = clicked.filter(|&k| k != BADGE) {
            self.selected.clear();
            self.selected.insert(k);
        }
        let Some(key) = self.selected.iter().next().copied() else { return };
        let layer = self.draft.current_mut();
        card(ui, icon::CURSOR_CLICK, &format!("{} key", keybed::label(key)), |ui| key_editor(ui, layer, key));
    }
}

fn key_editor(ui: &mut Ui, layer: &mut Layer, key: u8) {
    if layer.advanced.contains_key(&key) {
        ui.label(
            RichText::new(
                "This key has an advanced function (DKS, mod-tap, toggle or macro). Remapping it here replaces that.",
            )
            .color(WARN),
        );
    }
    let current = layer.bindings.get(&key).map(|c| c[0][2]).or_else(|| state::default_code(key).ok());
    ui.horizontal(|ui| {
        row_label(ui, "Sends", "");
        let text = current.map_or("-".into(), code_name);
        egui::ComboBox::from_id_salt("kb-code").selected_text(text).width(220.0).height(360.0).show_ui(ui, |ui| {
            for (name, code) in keycodes() {
                if ui.selectable_label(current == Some(code), name).clicked() {
                    layer.advanced.remove(&key);
                    if state::default_code(key).ok() == Some(code) {
                        layer.bindings.remove(&key);
                    } else {
                        layer.bindings.insert(key, [[0, 0, code, 0], [0; 4], [0; 4], [0; 4]]);
                    }
                }
            }
        });
        if ui.button(format!("{}  Default", icon::ARROW_COUNTER_CLOCKWISE)).clicked() {
            layer.bindings.remove(&key);
            layer.advanced.remove(&key);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{code_name, key_display, keycodes};

    #[test]
    fn key_names_are_readable() {
        assert_eq!(key_display("KeyA"), "A");
        assert_eq!(key_display("Digit1"), "1");
        assert_eq!(key_display("ArrowLeft"), "Arrow Left");
        assert_eq!(key_display("mouse_left"), "Mouse Left");
        assert_eq!(key_display("Custom_Fnkey"), "Custom Fnkey");
        assert_eq!(code_name(0xff), "0xff");
    }

    #[test]
    fn keycodes_are_unique_and_bindable() {
        let codes = keycodes();
        let unique: std::collections::BTreeSet<u8> = codes.iter().map(|(_, c)| *c).collect();
        assert_eq!(unique.len(), codes.len());
        assert!(!unique.contains(&0));
    }
}
