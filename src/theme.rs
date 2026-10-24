//! Palette, fonts and small custom widgets for the GUI.
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontFamily, FontId, Rect, Response, RichText, Sense, Stroke, TextStyle, Ui,
};
use std::sync::Arc;

pub const BG: Color32 = Color32::from_rgb(0x13, 0x14, 0x17);
pub const PANEL: Color32 = Color32::from_rgb(0x0D, 0x0E, 0x10);
pub const SURFACE: Color32 = Color32::from_rgb(0x1B, 0x1D, 0x21);
pub const SURFACE2: Color32 = Color32::from_rgb(0x24, 0x27, 0x2C);
pub const SURFACE3: Color32 = Color32::from_rgb(0x30, 0x34, 0x3A);
pub const BORDER: Color32 = Color32::from_rgb(0x2B, 0x2E, 0x34);
pub const TEXT: Color32 = Color32::from_rgb(0xEC, 0xEA, 0xE6);
pub const DIM: Color32 = Color32::from_rgb(0x9C, 0x9F, 0xA6);
pub const FAINT: Color32 = Color32::from_rgb(0x6B, 0x6F, 0x76);
pub const ACCENT: Color32 = Color32::from_rgb(0xF5, 0xA6, 0x23);
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x1E, 0x14, 0x04);
pub const OK: Color32 = Color32::from_rgb(0x5C, 0xC8, 0x7A);
pub const WARN: Color32 = Color32::from_rgb(0xF0, 0xB4, 0x4C);
pub const ERR: Color32 = Color32::from_rgb(0xEB, 0x6A, 0x5E);

pub fn heading_family() -> FontFamily {
    FontFamily::Name("heading".into())
}

fn strong_family() -> FontFamily {
    FontFamily::Name("strong".into())
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes) in [
        ("SpaceGrotesk-Medium", include_bytes!("../assets/fonts/SpaceGrotesk-Medium.ttf").as_slice()),
        ("SpaceGrotesk-Bold", include_bytes!("../assets/fonts/SpaceGrotesk-Bold.ttf").as_slice()),
        ("WorkSans-Medium", include_bytes!("../assets/fonts/WorkSans-Medium.ttf").as_slice()),
        ("WorkSans-SemiBold", include_bytes!("../assets/fonts/WorkSans-SemiBold.ttf").as_slice()),
        ("JetBrainsMono-Medium", include_bytes!("../assets/fonts/JetBrainsMono-Medium.ttf").as_slice()),
    ] {
        fonts.font_data.insert(name.into(), Arc::new(egui::FontData::from_static(bytes)));
    }
    // egui's own fonts stay behind ours as fallbacks for symbols the Latin subsets lack.
    let fallback = fonts.families[&FontFamily::Proportional].clone();
    for (family, font) in [
        (FontFamily::Proportional, "WorkSans-Medium"),
        (FontFamily::Monospace, "JetBrainsMono-Medium"),
        (strong_family(), "WorkSans-SemiBold"),
        (heading_family(), "SpaceGrotesk-Medium"),
        (FontFamily::Name("brand".into()), "SpaceGrotesk-Bold"),
    ] {
        fonts.families.entry(family).or_insert_with(|| fallback.clone()).insert(0, font.into());
    }
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);

    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |s| {
        s.text_styles = [
            (TextStyle::Heading, FontId::new(26.0, heading_family())),
            (TextStyle::Body, FontId::new(14.5, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(12.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(10.0, 10.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
        s.spacing.interact_size.y = 30.0;
        s.spacing.slider_width = 280.0;
        s.spacing.combo_width = 180.0;

        let v = &mut s.visuals;
        v.panel_fill = BG;
        v.window_fill = SURFACE;
        v.window_stroke = Stroke::new(1.0_f32, BORDER);
        v.window_corner_radius = CornerRadius::same(12);
        v.extreme_bg_color = PANEL;
        v.faint_bg_color = SURFACE2;
        v.override_text_color = None;
        v.hyperlink_color = ACCENT;
        v.slider_trailing_fill = true;
        v.selection.bg_fill = ACCENT;
        v.selection.stroke = Stroke::new(1.0_f32, ON_ACCENT);
        let radius = CornerRadius::same(8);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = SURFACE;
        w.noninteractive.weak_bg_fill = SURFACE;
        w.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
        w.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
        w.noninteractive.corner_radius = radius;
        for (state, fill, stroke) in [
            (&mut w.inactive, SURFACE2, Color32::TRANSPARENT),
            (&mut w.hovered, SURFACE3, ACCENT.gamma_multiply(0.5)),
            (&mut w.active, SURFACE3, ACCENT),
            (&mut w.open, SURFACE3, BORDER),
        ] {
            state.bg_fill = fill;
            state.weak_bg_fill = fill;
            state.bg_stroke = Stroke::new(1.0_f32, stroke);
            state.fg_stroke = Stroke::new(1.5_f32, TEXT);
            state.corner_radius = radius;
            state.expansion = 0.0;
        }
    });
}

/// A titled card with an icon.
pub fn card(ui: &mut Ui, icon: &str, title: &str, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new().fill(SURFACE).stroke(Stroke::new(1.0_f32, BORDER)).corner_radius(14).inner_margin(20).show(
        ui,
        |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon).size(18.0).color(ACCENT));
                ui.label(RichText::new(title).family(heading_family()).size(17.0).color(TEXT));
            });
            ui.add_space(10.0);
            add(ui);
        },
    );
    ui.add_space(14.0);
}

/// Page title with a subtitle and optional right-aligned actions.
pub fn page_header(ui: &mut Ui, title: &str, subtitle: &str, actions: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(title).heading().color(TEXT));
            ui.label(RichText::new(subtitle).color(DIM));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), actions);
    });
    ui.add_space(16.0);
}

/// A small rounded status badge.
pub fn pill(ui: &mut Ui, icon: &str, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.16))
        .stroke(Stroke::new(1.0_f32, color.gamma_multiply(0.35)))
        .corner_radius(20)
        .inner_margin(egui::Margin::symmetric(10, 3))
        .show(ui, |ui| {
            let label = if icon.is_empty() { text.to_string() } else { format!("{icon}  {text}") };
            ui.label(RichText::new(label).size(12.5).color(color));
        });
}

pub fn status_pill(ui: &mut Ui, outdated: bool) {
    use egui_phosphor::regular as i;
    if outdated {
        pill(ui, i::DOWNLOAD_SIMPLE, "Update available", WARN);
    } else {
        pill(ui, i::CHECK_CIRCLE, "Up to date", OK);
    }
}

/// Sidebar entry: icon + label, accent bar when selected.
pub fn nav_item(ui: &mut Ui, icon: &str, label: &str, selected: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), Sense::click());
    let painter = ui.painter();
    let bg = if selected {
        SURFACE2
    } else if resp.hovered() {
        SURFACE
    } else {
        Color32::TRANSPARENT
    };
    painter.rect_filled(rect, 10.0, bg);
    if selected {
        let bar = Rect::from_min_size(rect.left_top() + egui::vec2(0.0, 10.0), egui::vec2(3.0, 20.0));
        painter.rect_filled(bar, 2.0, ACCENT);
    }
    let color = if selected { TEXT } else { DIM };
    painter.text(
        rect.left_center() + egui::vec2(16.0, 0.0),
        Align2::LEFT_CENTER,
        icon,
        FontId::proportional(18.0),
        if selected { ACCENT } else { color },
    );
    painter.text(
        rect.left_center() + egui::vec2(46.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        FontId::new(15.0, FontFamily::Proportional),
        color,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// iOS-style switch.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let size = egui::vec2(44.0, 24.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let radius = rect.height() / 2.0;
    let bg = if *on { ACCENT } else { SURFACE3 };
    ui.painter().rect_filled(rect, radius, bg);
    let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
    ui.painter().circle_filled(egui::pos2(x, rect.center().y), radius - 4.0, if *on { ON_ACCENT } else { TEXT });
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A row of mutually exclusive buttons. Returns true when the value changed.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let before = *value;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (v, label) in options {
            let selected = *value == *v;
            let text = RichText::new(*label).color(if selected { ON_ACCENT } else { TEXT });
            let button =
                egui::Button::new(text).fill(if selected { ACCENT } else { SURFACE2 }).min_size(egui::vec2(56.0, 30.0));
            if ui.add(button).clicked() {
                *value = *v;
            }
        }
    });
    *value != before
}

/// Primary call-to-action button.
pub fn primary(ui: &mut Ui, enabled: bool, text: &str) -> bool {
    let button = egui::Button::new(RichText::new(text).family(strong_family()).color(ON_ACCENT))
        .fill(ACCENT)
        .min_size(egui::vec2(120.0, 34.0));
    ui.add_enabled(enabled, button).clicked()
}

/// Label column of a settings row.
pub fn row_label(ui: &mut Ui, title: &str, hint: &str) {
    ui.vertical(|ui| {
        ui.set_min_width(190.0);
        ui.label(RichText::new(title).color(TEXT));
        if !hint.is_empty() {
            ui.label(RichText::new(hint).size(12.0).color(FAINT));
        }
    });
}

/// Ring gauge with the value in the middle.
pub fn ring(ui: &mut Ui, fraction: f32, size: f32, center_text: &str, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), Sense::hover());
    let c = rect.center();
    let r = size / 2.0 - 6.0;
    let painter = ui.painter();
    painter.circle_stroke(c, r, Stroke::new(8.0_f32, SURFACE3));
    let n = 64;
    let end = fraction.clamp(0.0, 1.0) * std::f32::consts::TAU;
    let points: Vec<egui::Pos2> = (0..=n)
        .map(|k| {
            let a = -std::f32::consts::FRAC_PI_2 + end * k as f32 / n as f32;
            c + r * egui::vec2(a.cos(), a.sin())
        })
        .collect();
    if fraction > 0.0 {
        painter.add(egui::Shape::line(points, Stroke::new(8.0_f32, color)));
    }
    painter.text(c, Align2::CENTER_CENTER, center_text, FontId::new(size * 0.22, heading_family()), TEXT);
}

/// Stat tile used on the overview.
pub fn tile(ui: &mut Ui, icon: &str, label: &str, value: &str, sub: &str) {
    egui::Frame::new().fill(SURFACE).stroke(Stroke::new(1.0_f32, BORDER)).corner_radius(14).inner_margin(16).show(
        ui,
        |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon).size(16.0).color(ACCENT));
                ui.label(RichText::new(label).size(12.5).color(DIM));
            });
            ui.label(RichText::new(value).family(heading_family()).size(22.0).color(TEXT));
            ui.label(RichText::new(sub).size(12.0).color(FAINT));
        },
    );
}

/// A clickable colour dot.
pub fn swatch(ui: &mut Ui, color: [u8; 3], selected: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), Sense::click());
    let c = Color32::from_rgb(color[0], color[1], color[2]);
    ui.painter().circle_filled(rect.center(), 11.0, c);
    if selected || resp.hovered() {
        ui.painter().circle_stroke(rect.center(), 12.5, Stroke::new(2.0_f32, if selected { TEXT } else { DIM }));
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// Highlighted note inside a page.
pub fn info_banner(ui: &mut Ui, text: &str) {
    egui::Frame::new()
        .fill(ACCENT.gamma_multiply(0.08))
        .stroke(Stroke::new(1.0_f32, ACCENT.gamma_multiply(0.3)))
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).color(TEXT));
        });
}
