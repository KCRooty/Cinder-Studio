//! Marco común de los dos visores: cabecera con el nombre y el código de tiempo, la imagen,
//! la barra de posición con el tramo marcado y los botones de transporte.

use eframe::egui::{
    self, Align2, Color32, ColorImage, FontId, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, Vec2,
};

use crate::icons::{self, Icon};
use crate::theme;

const HEADER_H: f32 = 26.0;
const SCRUB_H: f32 = 16.0;
const TRANSPORT_H: f32 = 34.0;

pub struct MonitorInput<'a> {
    /// «Origen» o «Programa».
    pub kind: &'a str,
    /// Nombre del clip o de la línea de tiempo.
    pub title: &'a str,
    pub timecode: String,
    pub duration: String,
    pub texture: Option<&'a TextureHandle>,
    pub playing: bool,
    /// Posición de 0 a 1.
    pub position: f64,
    /// Tramo marcado, de 0 a 1.
    pub marks: Option<(f64, f64)>,
    /// Es el visor que recibe el teclado.
    pub active: bool,
    /// Botones de marcar entrada y salida.
    pub with_marks: bool,
    /// Texto sobre la imagen cuando no hay nada que enseñar.
    pub empty: Option<&'a str>,
    /// Aviso pequeño junto a la duración (sin sonido, error…).
    pub note: Option<(&'a str, Color32)>,
}

#[derive(Default)]
pub struct MonitorOutput {
    pub clicked: bool,
    /// Se arrastró la barra de posición hasta aquí (0 a 1).
    pub scrub: Option<f64>,
    pub to_start: bool,
    pub step_back: bool,
    pub toggle: bool,
    pub step_fwd: bool,
    pub to_end: bool,
    pub mark_in: bool,
    pub mark_out: bool,
}

pub fn show(ui: &mut egui::Ui, m: MonitorInput<'_>) -> MonitorOutput {
    let mut out = MonitorOutput::default();
    let rect = ui.available_rect_before_wrap();
    // Reserva todo el hueco; las zonas con las que se interactúa se colocan encima.
    ui.allocate_rect(rect, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::PANEL);

    // Cabecera.
    let header = Rect::from_min_size(rect.min, Vec2::new(rect.width(), HEADER_H));
    if m.active {
        painter.rect_filled(Rect::from_min_size(header.min, Vec2::new(header.width(), 2.0)), 0.0, theme::HOT);
    }
    painter.text(
        header.left_center() + Vec2::new(10.0, 1.0),
        Align2::LEFT_CENTER,
        m.kind,
        FontId::proportional(11.0),
        theme::TEXT_DIM,
    );
    painter.text(
        header.center() + Vec2::new(0.0, 1.0),
        Align2::CENTER_CENTER,
        m.title,
        FontId::proportional(12.5),
        if m.active { theme::TEXT_BRIGHT } else { theme::TEXT },
    );
    painter.text(
        header.right_center() + Vec2::new(-10.0, 1.0),
        Align2::RIGHT_CENTER,
        &m.timecode,
        FontId::monospace(13.0),
        theme::TEXT_BRIGHT,
    );

    // Imagen.
    let picture = Rect::from_min_max(
        Pos2::new(rect.left(), header.bottom()),
        Pos2::new(rect.right(), (rect.bottom() - SCRUB_H - TRANSPORT_H).max(header.bottom() + 1.0)),
    );
    painter.rect_filled(picture, 0.0, theme::BG);
    if let Some(tex) = m.texture {
        let size = tex.size_vec2();
        if size.x > 0.0 && size.y > 0.0 {
            let room = picture.shrink(6.0);
            let scale = (room.width() / size.x).min(room.height() / size.y);
            let fit = Rect::from_center_size(room.center(), size * scale);
            let uv = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));
            painter.image(tex.id(), fit, uv, Color32::WHITE);
        }
    } else if let Some(text) = m.empty {
        painter.text(picture.center(), Align2::CENTER_CENTER, text, FontId::proportional(12.5), theme::TEXT_DIM);
    }
    if ui.allocate_rect(picture, Sense::click()).clicked() {
        out.clicked = true;
    }

    // Barra de posición.
    let scrub = Rect::from_min_size(Pos2::new(rect.left(), picture.bottom()), Vec2::new(rect.width(), SCRUB_H));
    let track = Rect::from_min_max(
        Pos2::new(scrub.left() + 10.0, scrub.center().y - 2.0),
        Pos2::new(scrub.right() - 10.0, scrub.center().y + 2.0),
    );
    painter.rect_filled(track, 2.0, theme::BG);
    let x_of = |t: f64| track.left() + track.width() * t.clamp(0.0, 1.0) as f32;
    if let Some((a, b)) = m.marks {
        let band = Rect::from_min_max(Pos2::new(x_of(a), track.top()), Pos2::new(x_of(b), track.bottom()));
        painter.rect_filled(band, 0.0, theme::TEXT_DIM);
        for x in [x_of(a), x_of(b)] {
            painter.line_segment(
                [Pos2::new(x, scrub.top() + 2.0), Pos2::new(x, scrub.bottom() - 2.0)],
                Stroke::new(1.5, theme::TEXT_BRIGHT),
            );
        }
    }
    let x = x_of(m.position);
    painter.line_segment(
        [Pos2::new(x, scrub.top() + 1.0), Pos2::new(x, scrub.bottom() - 1.0)],
        Stroke::new(2.0, theme::HOT),
    );
    let resp = ui.allocate_rect(scrub, Sense::click_and_drag());
    if (resp.dragged() || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
    {
        out.scrub = Some(f64::from(((p.x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0)));
        out.clicked = true;
    }

    // Transporte.
    let bar = Rect::from_min_max(Pos2::new(rect.left(), scrub.bottom()), rect.max);
    painter.text(
        bar.left_center() + Vec2::new(10.0, 0.0),
        Align2::LEFT_CENTER,
        &m.duration,
        FontId::monospace(11.0),
        theme::TEXT_DIM,
    );
    let size = 26.0;
    if let Some((note, color)) = m.note {
        // Hasta donde empiezan los botones, sin pisarlos.
        let room = Rect::from_min_max(bar.min, Pos2::new(bar.center().x - 2.5 * (size + 4.0) - 8.0, bar.max.y));
        painter.with_clip_rect(room).text(
            bar.left_center() + Vec2::new(92.0, 0.0),
            Align2::LEFT_CENTER,
            note,
            FontId::proportional(11.0),
            color,
        );
    }
    let buttons = [
        (Icon::ToStart, "Ir al principio (Inicio)"),
        (Icon::StepBack, "Un fotograma atrás (←)"),
        (if m.playing { Icon::Pause } else { Icon::Play }, "Reproducir o parar (Espacio · K / L)"),
        (Icon::StepFwd, "Un fotograma adelante (→)"),
        (Icon::ToEnd, "Ir al final (Fin)"),
    ];
    let total = buttons.len() as f32 * (size + 4.0);
    let mut bx = bar.center().x - total / 2.0;
    for (i, (icon, tip)) in buttons.into_iter().enumerate() {
        let r = Rect::from_min_size(Pos2::new(bx, bar.center().y - size / 2.0), Vec2::splat(size));
        if icons::button_at(ui, r, icon, false, false).on_hover_text(tip).clicked() {
            match i {
                0 => out.to_start = true,
                1 => out.step_back = true,
                2 => out.toggle = true,
                3 => out.step_fwd = true,
                _ => out.to_end = true,
            }
            out.clicked = true;
        }
        bx += size + 4.0;
    }
    if m.with_marks {
        let r_out =
            Rect::from_min_size(Pos2::new(bar.right() - 10.0 - size, bar.center().y - size / 2.0), Vec2::splat(size));
        let r_in = r_out.translate(Vec2::new(-(size + 4.0), 0.0));
        if icons::button_at(ui, r_in, Icon::MarkIn, false, false).on_hover_text("Marcar entrada (I)").clicked() {
            out.mark_in = true;
            out.clicked = true;
        }
        if icons::button_at(ui, r_out, Icon::MarkOut, false, false).on_hover_text("Marcar salida (O)").clicked() {
            out.mark_out = true;
            out.clicked = true;
        }
    }
    out
}

/// Sube un fotograma a la textura (la crea la primera vez).
pub fn upload(ctx: &egui::Context, texture: &mut Option<TextureHandle>, name: &str, frame: &cinder_media::VideoFrame) {
    let image = ColorImage::from_rgba_unmultiplied([frame.width as usize, frame.height as usize], &frame.rgba);
    match texture.as_mut() {
        Some(t) => t.set(image, TextureOptions::LINEAR),
        None => *texture = Some(ctx.load_texture(name, image, TextureOptions::LINEAR)),
    }
}

/// HH:MM:SS:FF, como en cualquier editor.
pub fn timecode(secs: f64, fps: f64) -> String {
    let fps = if fps > 0.0 { fps } else { 25.0 };
    let base = fps.round().max(1.0) as u64;
    let total = (secs.max(0.0) * fps).round() as u64;
    let f = total % base;
    let s = total / base;
    format!("{:02}:{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60, f)
}

#[cfg(test)]
mod tests {
    use super::timecode;

    #[test]
    fn timecode_cuenta_fotogramas() {
        assert_eq!(timecode(0.0, 25.0), "00:00:00:00");
        assert_eq!(timecode(1.24, 25.0), "00:00:01:06");
        assert_eq!(timecode(3661.48, 25.0), "01:01:01:12");
        assert_eq!(timecode(2.0, 0.0), "00:00:02:00");
    }
}
