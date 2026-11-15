//! Iconos dibujados con el pintor de egui. Son propios (AGENTS.md §3): formas simples hechas
//! a mano, sin fuentes de iconos ni assets de nadie.

use eframe::egui::{self, Color32, Painter, Pos2, Rect, Sense, Shape, Stroke, Vec2};

use crate::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Play,
    Pause,
    ToStart,
    ToEnd,
    StepBack,
    StepFwd,
    MarkIn,
    MarkOut,
    Arrow,
    Blade,
    Insert,
    Overwrite,
    Append,
    Magnet,
    Link,
    Eye,
    Speaker,
    Plus,
    Minus,
    Fit,
    Folder,
    // Páginas
    Film,
    Scissors,
    Tracks,
    Nodes,
    Wheel,
    Wave,
    Export,
}

/// Dibuja el icono centrado en `rect`. `off` lo tacha (pista silenciada u oculta).
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32, off: bool) {
    let s = rect.width().min(rect.height()) * 0.5;
    let c = rect.center();
    let p = |x: f32, y: f32| Pos2::new(c.x + x * s, c.y + y * s);
    let line = Stroke::new((s * 0.16).max(1.2), color);
    let fill = |pts: Vec<Pos2>| painter.add(Shape::convex_polygon(pts, color, Stroke::NONE));
    let bar =
        |x0: f32, y0: f32, x1: f32, y1: f32| painter.rect_filled(Rect::from_two_pos(p(x0, y0), p(x1, y1)), 0.5, color);
    match icon {
        Icon::Play => {
            fill(vec![p(-0.5, -0.7), p(-0.5, 0.7), p(0.7, 0.0)]);
        }
        Icon::Pause => {
            bar(-0.55, -0.65, -0.15, 0.65);
            bar(0.15, -0.65, 0.55, 0.65);
        }
        Icon::ToStart => {
            bar(-0.75, -0.6, -0.5, 0.6);
            fill(vec![p(0.7, -0.6), p(0.7, 0.6), p(-0.4, 0.0)]);
        }
        Icon::ToEnd => {
            bar(0.5, -0.6, 0.75, 0.6);
            fill(vec![p(-0.7, -0.6), p(-0.7, 0.6), p(0.4, 0.0)]);
        }
        Icon::StepBack => {
            fill(vec![p(0.15, -0.6), p(0.15, 0.6), p(-0.7, 0.0)]);
            bar(0.4, -0.6, 0.62, 0.6);
        }
        Icon::StepFwd => {
            fill(vec![p(-0.15, -0.6), p(-0.15, 0.6), p(0.7, 0.0)]);
            bar(-0.62, -0.6, -0.4, 0.6);
        }
        Icon::MarkIn => {
            painter.add(Shape::line(vec![p(0.2, -0.7), p(-0.45, -0.7), p(-0.45, 0.7), p(0.2, 0.7)], line));
            fill(vec![p(-0.1, -0.3), p(-0.1, 0.3), p(0.45, 0.0)]);
        }
        Icon::MarkOut => {
            painter.add(Shape::line(vec![p(-0.2, -0.7), p(0.45, -0.7), p(0.45, 0.7), p(-0.2, 0.7)], line));
            fill(vec![p(0.1, -0.3), p(0.1, 0.3), p(-0.45, 0.0)]);
        }
        Icon::Arrow => {
            fill(vec![p(-0.45, -0.8), p(-0.45, 0.45), p(0.5, 0.15)]);
            fill(vec![p(-0.12, 0.2), p(0.14, 0.08), p(0.42, 0.7), p(0.16, 0.82)]);
        }
        Icon::Blade => {
            // Cuchilla: un rombo alargado con la ranura del centro.
            let pts = vec![p(-0.15, -0.85), p(0.75, 0.05), p(0.15, 0.85), p(-0.75, -0.05)];
            painter.add(Shape::closed_line(pts, line));
            painter.line_segment([p(-0.2, -0.2), p(0.2, 0.2)], line);
        }
        Icon::Insert => {
            bar(-0.8, 0.3, -0.15, 0.7);
            bar(0.15, 0.3, 0.8, 0.7);
            fill(vec![p(-0.35, -0.7), p(0.35, -0.7), p(0.0, 0.1)]);
        }
        Icon::Overwrite => {
            bar(-0.8, 0.3, 0.8, 0.7);
            fill(vec![p(-0.35, -0.7), p(0.35, -0.7), p(0.0, 0.1)]);
        }
        Icon::Append => {
            bar(-0.8, -0.2, 0.1, 0.2);
            fill(vec![p(0.3, -0.5), p(0.3, 0.5), p(0.85, 0.0)]);
        }
        Icon::Magnet => {
            let thick = Stroke::new((s * 0.34).max(2.0), color);
            let mut pts = vec![p(-0.5, -0.7), p(-0.5, 0.05)];
            for i in 0..=8 {
                let a = std::f32::consts::PI * (1.0 - i as f32 / 8.0);
                pts.push(p(0.5 * a.cos(), 0.05 + 0.5 * a.sin()));
            }
            pts.push(p(0.5, -0.7));
            painter.add(Shape::line(pts, thick));
        }
        Icon::Link => {
            painter.circle_stroke(p(-0.3, 0.0), s * 0.42, line);
            painter.circle_stroke(p(0.3, 0.0), s * 0.42, line);
        }
        Icon::Eye => {
            let mut pts = Vec::new();
            for i in 0..=12 {
                let t = i as f32 / 12.0;
                pts.push(p(-0.85 + 1.7 * t, -0.5 * (std::f32::consts::PI * t).sin()));
            }
            for i in 0..=12 {
                let t = 1.0 - i as f32 / 12.0;
                pts.push(p(-0.85 + 1.7 * t, 0.5 * (std::f32::consts::PI * t).sin()));
            }
            painter.add(Shape::closed_line(pts, line));
            painter.circle_filled(c, s * 0.22, color);
        }
        Icon::Speaker => {
            bar(-0.75, -0.25, -0.4, 0.25);
            fill(vec![p(-0.4, -0.25), p(0.05, -0.65), p(0.05, 0.65), p(-0.4, 0.25)]);
            let mut arc = Vec::new();
            for i in 0..=8 {
                let a = -0.9 + 1.8 * i as f32 / 8.0;
                arc.push(p(0.15 + 0.5 * a.cos(), 0.5 * a.sin()));
            }
            painter.add(Shape::line(arc, line));
        }
        Icon::Plus => {
            bar(-0.7, -0.1, 0.7, 0.1);
            bar(-0.1, -0.7, 0.1, 0.7);
        }
        Icon::Minus => {
            bar(-0.7, -0.1, 0.7, 0.1);
        }
        Icon::Fit => {
            bar(-0.85, -0.6, -0.7, 0.6);
            bar(0.7, -0.6, 0.85, 0.6);
            bar(-0.35, -0.08, 0.35, 0.08);
            fill(vec![p(-0.6, 0.0), p(-0.25, -0.3), p(-0.25, 0.3)]);
            fill(vec![p(0.6, 0.0), p(0.25, -0.3), p(0.25, 0.3)]);
        }
        Icon::Folder => {
            fill(vec![p(-0.8, -0.55), p(-0.2, -0.55), p(0.0, -0.3), p(-0.8, -0.3)]);
            bar(-0.8, -0.3, 0.8, 0.6);
        }
        Icon::Film => {
            painter.rect_stroke(Rect::from_two_pos(p(-0.8, -0.6), p(0.8, 0.6)), 1.0, line, egui::StrokeKind::Inside);
            for x in [-0.5, 0.0, 0.5] {
                bar(x - 0.09, -0.48, x + 0.09, -0.3);
                bar(x - 0.09, 0.3, x + 0.09, 0.48);
            }
        }
        Icon::Scissors => {
            painter.line_segment([p(-0.6, -0.7), p(0.35, 0.35)], line);
            painter.line_segment([p(0.6, -0.7), p(-0.35, 0.35)], line);
            painter.circle_stroke(p(-0.48, 0.55), s * 0.22, line);
            painter.circle_stroke(p(0.48, 0.55), s * 0.22, line);
        }
        Icon::Tracks => {
            bar(-0.8, -0.65, 0.3, -0.35);
            bar(-0.4, -0.15, 0.8, 0.15);
            bar(-0.8, 0.35, 0.0, 0.65);
        }
        Icon::Nodes => {
            painter.line_segment([p(-0.45, -0.4), p(0.45, 0.0)], line);
            painter.line_segment([p(-0.45, 0.45), p(0.45, 0.0)], line);
            bar(-0.8, -0.65, -0.3, -0.2);
            bar(-0.8, 0.2, -0.3, 0.65);
            bar(0.3, -0.25, 0.8, 0.25);
        }
        Icon::Wheel => {
            painter.circle_stroke(c, s * 0.75, line);
            painter.circle_filled(p(0.2, -0.15), s * 0.16, color);
            painter.line_segment([c, p(0.2, -0.15)], Stroke::new(1.0, color));
        }
        Icon::Wave => {
            for (i, h) in [0.3, 0.7, 0.45, 0.85, 0.35, 0.6, 0.25].into_iter().enumerate() {
                let x = -0.78 + i as f32 * 0.26;
                bar(x - 0.07, -h, x + 0.07, h);
            }
        }
        Icon::Export => {
            painter.add(Shape::line(vec![p(-0.7, 0.0), p(-0.7, 0.7), p(0.7, 0.7), p(0.7, 0.0)], line));
            bar(-0.09, -0.35, 0.09, 0.35);
            fill(vec![p(-0.4, -0.3), p(0.4, -0.3), p(0.0, -0.8)]);
        }
    }
    if off {
        painter.line_segment([p(-0.8, 0.8), p(0.8, -0.8)], Stroke::new((s * 0.2).max(1.5), theme::HOT));
    }
}

fn draw_button(ui: &egui::Ui, rect: Rect, resp: &egui::Response, icon: Icon, lit: bool, off: bool) {
    let painter = ui.painter();
    if lit {
        painter.rect_filled(rect, 3.0, theme::RAISED_HOVER);
    } else if resp.hovered() {
        painter.rect_filled(rect, 3.0, theme::RAISED);
    }
    let color = if lit || resp.hovered() { theme::TEXT_BRIGHT } else { theme::TEXT_DIM };
    let inner = Rect::from_center_size(rect.center(), Vec2::splat(rect.height().min(rect.width()) * 0.62));
    paint(painter, inner, icon, color, off);
}

/// Botón de icono en un rectángulo dado (para las zonas que se colocan a mano). `lit` lo deja
/// encendido (herramienta activa) y `off` lo tacha.
pub fn button_at(ui: &mut egui::Ui, rect: Rect, icon: Icon, lit: bool, off: bool) -> egui::Response {
    let resp = ui.allocate_rect(rect, Sense::click());
    draw_button(ui, rect, &resp, icon, lit, off);
    resp
}

/// Botón de icono dentro de un layout normal.
pub fn button(ui: &mut egui::Ui, icon: Icon, lit: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
    draw_button(ui, rect, &resp, icon, lit, false);
    resp
}
