//! Aspecto de la aplicación: carbón neutro con poco contraste entre paneles, texto pequeño y
//! un único color vivo para el cabezal y la selección, como en las suites de posproducción.
//! Los colores son propios; nada sale de capturas ni de assets de terceros (AGENTS.md §3).

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle};

/// Lo más oscuro: detrás de la imagen de los visores y en los carriles de la línea de tiempo.
pub const BG: Color32 = Color32::from_rgb(0x19, 0x19, 0x1C);
/// Fondo de los paneles.
pub const PANEL: Color32 = Color32::from_rgb(0x25, 0x25, 0x2A);
/// Cabeceras, botones y casillas sobre un panel.
pub const RAISED: Color32 = Color32::from_rgb(0x31, 0x31, 0x38);
pub const RAISED_HOVER: Color32 = Color32::from_rgb(0x3D, 0x3D, 0x46);
/// Líneas entre paneles.
pub const LINE: Color32 = Color32::from_rgb(0x0F, 0x0F, 0x12);
/// Carril de una pista.
pub const LANE: Color32 = Color32::from_rgb(0x20, 0x20, 0x24);

pub const TEXT: Color32 = Color32::from_rgb(0xC6, 0xC6, 0xCC);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x86, 0x86, 0x90);
pub const TEXT_BRIGHT: Color32 = Color32::from_rgb(0xF2, 0xF2, 0xF4);

/// Cabezal, selección y pista de destino.
pub const HOT: Color32 = Color32::from_rgb(0xE8, 0x4C, 0x3D);
/// El violeta de la marca Cinder (solo para el nombre).
pub const BRAND: Color32 = Color32::from_rgb(0x7B, 0x61, 0xFF);
pub const WARN: Color32 = Color32::from_rgb(0xF0, 0xA8, 0x3C);
pub const ERROR: Color32 = Color32::from_rgb(0xF0, 0x7F, 0x7F);

pub const CLIP_VIDEO: Color32 = Color32::from_rgb(0x4C, 0x78, 0xA8);
pub const CLIP_VIDEO_DARK: Color32 = Color32::from_rgb(0x38, 0x5B, 0x82);
pub const CLIP_AUDIO: Color32 = Color32::from_rgb(0x3D, 0x8A, 0x62);
pub const CLIP_AUDIO_DARK: Color32 = Color32::from_rgb(0x2C, 0x68, 0x49);
pub const WAVE: Color32 = Color32::from_rgb(0xA8, 0xE6, 0xC4);

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.global_style()).clone();
    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.extreme_bg_color = BG;
    v.faint_bg_color = RAISED;
    v.override_text_color = None;
    v.selection.bg_fill = Color32::from_rgb(0x4A, 0x4A, 0x56);
    v.selection.stroke = Stroke::new(1.0, TEXT_BRIGHT);
    v.hyperlink_color = TEXT_BRIGHT;
    v.window_stroke = Stroke::new(1.0, LINE);
    let r = CornerRadius::same(3);
    v.widgets.noninteractive.bg_fill = PANEL;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.bg_fill = RAISED;
    v.widgets.inactive.weak_bg_fill = RAISED;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.bg_fill = RAISED_HOVER;
    v.widgets.hovered.weak_bg_fill = RAISED_HOVER;
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT_BRIGHT);
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.bg_fill = RAISED_HOVER;
    v.widgets.active.weak_bg_fill = RAISED_HOVER;
    v.widgets.active.bg_stroke = Stroke::NONE;
    v.widgets.active.fg_stroke = Stroke::new(1.0, TEXT_BRIGHT);
    v.widgets.active.corner_radius = r;
    v.widgets.open.bg_fill = RAISED_HOVER;
    v.widgets.open.weak_bg_fill = RAISED_HOVER;
    v.widgets.open.corner_radius = r;

    style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    style.spacing.button_padding = egui::vec2(8.0, 3.0);
    style.spacing.interact_size.y = 22.0;
    style.text_styles = [
        (TextStyle::Small, FontId::new(10.5, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(12.5, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(12.5, FontFamily::Proportional)),
        (TextStyle::Heading, FontId::new(15.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace)),
    ]
    .into();
    ctx.set_global_style(style);
}

/// Marco de un panel sin margen, para dibujar hasta el borde.
pub fn flush(fill: Color32) -> egui::Frame {
    egui::Frame::new().fill(fill).inner_margin(0.0)
}
