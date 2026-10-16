//! PolyRoom's dark theme: neutral greys so models stand out, one warm accent.

use egui::{Color32, CornerRadius, Stroke, Visuals};

pub const PANEL: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x2c);
pub const CENTER: Color32 = Color32::from_rgb(0x1c, 0x1c, 0x1e);
pub const HEADER: Color32 = Color32::from_rgb(0x22, 0x22, 0x24);
pub const TEXT: Color32 = Color32::from_rgb(0xc8, 0xc8, 0xcc);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x8a, 0x8a, 0x90);
pub const ACCENT: Color32 = Color32::from_rgb(0xe8, 0x91, 0x4a);

pub fn apply(ctx: &egui::Context) {
    let mut v = Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.extreme_bg_color = CENTER;
    v.faint_bg_color = HEADER;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT.gamma_multiply(0.6);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(3);
    }
    // PolyRoom is always dark, whatever the OS theme, so models read consistently.
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.set_theme(egui::ThemePreference::Dark);
}
