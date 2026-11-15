//! Línea de tiempo de la página Edición: regla con el cabezal, pistas y clips.
//!
//! Ratón: clic en la regla o en un hueco mueve el cabezal · clic en un clip lo selecciona ·
//! arrastrar un clip lo mueve (también a otra pista del mismo tipo) · arrastrar un borde lo
//! recorta · Ctrl+rueda acerca o aleja · rueda desplaza. Al arrastrar se pega al cabezal y a
//! los bordes de los otros clips.

use cinder_core::{Clip, ClipId, Edit, Frames, Project, TrackKind, ops};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

use crate::ACCENT;
use crate::viewer::timecode;

const HEADER_W: f32 = 64.0;
const RULER_H: f32 = 24.0;
const VIDEO_H: f32 = 52.0;
const AUDIO_H: f32 = 40.0;
/// Distancia en píxeles a la que un borde se pega a otro al arrastrar.
const SNAP_PX: f32 = 8.0;
/// Zona de cada borde del clip que recorta en vez de mover.
const EDGE_PX: f32 = 6.0;

#[derive(Debug, Clone, Copy, PartialEq)]
enum DragKind {
    Move,
    TrimStart,
    TrimEnd,
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    track: usize,
    id: ClipId,
    kind: DragKind,
    /// Fotogramas desplazados desde que empezó el arrastre (ya pegados).
    delta: Frames,
    /// Pista sobre la que está el ratón (solo para mover).
    target: usize,
}

pub struct TimelineView {
    /// Píxeles por fotograma.
    zoom: f32,
    /// Primer fotograma visible.
    scroll: f32,
    pub selected: Option<(usize, ClipId)>,
    drag: Option<Drag>,
    /// Fila de cada pista en pantalla (índice de pista, rectángulo), del último dibujado.
    rows: Vec<(usize, Rect)>,
}

/// Lo que pide la línea de tiempo tras un repintado.
#[derive(Default)]
pub struct Response {
    pub seek: Option<Frames>,
    pub edit: Option<Edit>,
    /// Un recorte imposible (se sale del medio, por ejemplo).
    pub error: Option<cinder_core::EditError>,
    /// Se tocó la línea de tiempo: el visor de programa pasa a ser el activo.
    pub focused: bool,
}

impl TimelineView {
    pub fn new() -> Self {
        Self { zoom: 4.0, scroll: 0.0, selected: None, drag: None, rows: Vec::new() }
    }

    pub fn zoom_by(&mut self, factor: f32) {
        self.zoom = (self.zoom * factor).clamp(0.02, 40.0);
    }

    /// Encaja la línea de tiempo entera en el ancho visible.
    pub fn fit(&mut self, duration: Frames, width: f32) {
        if duration > 0 {
            self.zoom = ((width - HEADER_W - 20.0) / duration as f32).clamp(0.02, 40.0);
            self.scroll = 0.0;
        }
    }

    /// Orden en pantalla: vídeo de arriba abajo (V2 sobre V1) y luego audio (A1, A2…).
    fn row_order(project: &Project) -> Vec<usize> {
        let tracks = &project.timeline.tracks;
        let mut video: Vec<usize> = (0..tracks.len()).filter(|&i| tracks[i].kind == TrackKind::Video).collect();
        video.reverse();
        let audio = (0..tracks.len()).filter(|&i| tracks[i].kind == TrackKind::Audio);
        video.into_iter().chain(audio).collect()
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, project: &Project, playhead: Frames, width_hint: &mut f32) -> Response {
        let mut out = Response::default();
        let fps = project.settings.fps.as_f64();
        let full = ui.available_rect_before_wrap();
        *width_hint = full.width();
        let (rect, bg) = ui.allocate_exact_size(full.size(), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_gray(22));
        let lane_left = rect.left() + HEADER_W;

        // Rueda: Ctrl acerca/aleja alrededor del ratón; sin Ctrl desplaza.
        if bg.hovered() {
            let (scroll, ctrl, pointer) =
                ui.input(|i| (i.smooth_scroll_delta, i.modifiers.command, i.pointer.hover_pos()));
            if ctrl && scroll.y != 0.0 {
                let anchor = pointer.map_or(self.scroll, |p| (p.x - lane_left) / self.zoom + self.scroll);
                self.zoom_by((scroll.y / 200.0).exp());
                if let Some(p) = pointer {
                    self.scroll = anchor - (p.x - lane_left) / self.zoom;
                }
            } else if scroll.x != 0.0 || scroll.y != 0.0 {
                let d = if scroll.x != 0.0 { scroll.x } else { scroll.y };
                self.scroll -= d / self.zoom;
            }
            self.scroll = self.scroll.max(0.0);
        }

        let (scroll, zoom) = (self.scroll, self.zoom);
        let x_of = |f: Frames| lane_left + (f as f32 - scroll) * zoom;
        let frame_of = |x: f32| ((x - lane_left) / zoom + scroll).round() as Frames;

        // Regla.
        let ruler = Rect::from_min_size(Pos2::new(lane_left, rect.top()), Vec2::new(rect.width() - HEADER_W, RULER_H));
        painter.rect_filled(ruler, 0.0, Color32::from_gray(30));
        let step = ruler_step(self.zoom, fps);
        let first = (self.scroll as Frames / step) * step;
        let mut f = first;
        while x_of(f) < rect.right() {
            let x = x_of(f);
            if x >= lane_left {
                painter.line_segment(
                    [Pos2::new(x, ruler.bottom() - 6.0), Pos2::new(x, ruler.bottom())],
                    Stroke::new(1.0, Color32::GRAY),
                );
                painter.text(
                    Pos2::new(x + 3.0, ruler.top() + 3.0),
                    Align2::LEFT_TOP,
                    timecode(f as f64 / fps, fps),
                    FontId::monospace(10.0),
                    Color32::GRAY,
                );
            }
            f += step;
        }

        // Pistas.
        self.rows.clear();
        let mut y = ruler.bottom() + 2.0;
        for i in Self::row_order(project) {
            let t = &project.timeline.tracks[i];
            let h = if t.kind == TrackKind::Video { VIDEO_H } else { AUDIO_H };
            let row = Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), h));
            self.rows.push((i, row));
            painter.rect_filled(row.shrink2(Vec2::new(0.0, 1.0)), 0.0, Color32::from_gray(28));
            let head = Rect::from_min_size(row.min, Vec2::new(HEADER_W - 4.0, h));
            painter.rect_filled(head.shrink(1.0), 2.0, Color32::from_gray(38));
            painter.text(
                head.left_center() + Vec2::new(8.0, 0.0),
                Align2::LEFT_CENTER,
                &t.name,
                FontId::proportional(13.0),
                Color32::LIGHT_GRAY,
            );
            y += h + 2.0;
        }

        let pointer = ui.input(|i| i.pointer.interact_pos());
        let row_at = |p: Pos2| self.rows.iter().find(|(_, r)| r.contains(p)).map(|(i, _)| *i);
        let rows = self.rows.clone();
        let row_rect = |track: usize| rows.iter().find(|(i, _)| *i == track).map(|(_, r)| *r);

        // Clips (y qué hay bajo el ratón, para empezar un arrastre).
        // Al empezar un arrastre el ratón ya se ha movido unos píxeles: lo que cuenta es dónde
        // se pulsó (si no, agarrar un borde acabaría moviendo el clip).
        let probe = if bg.drag_started() { ui.input(|i| i.pointer.press_origin()).or(pointer) } else { pointer };
        let mut hit: Option<(usize, ClipId, DragKind)> = None;
        let selection = self.selected.map(|(t, id)| ops::linked(project, t, id)).unwrap_or_default();
        for &(i, row) in &rows {
            let t = &project.timeline.tracks[i];
            for c in t.clips() {
                let r = Rect::from_min_max(
                    Pos2::new(x_of(c.start).max(lane_left), row.top() + 3.0),
                    Pos2::new(x_of(c.end()), row.bottom() - 3.0),
                );
                if r.right() < lane_left || r.left() > rect.right() {
                    continue;
                }
                let dragging_this = self.drag.is_some_and(|d| d.id == c.id);
                let selected = selection.contains(&(i, c.id));
                self.paint_clip(&painter, project, c, t.kind, r, selected, dragging_this);
                if let Some(p) = probe
                    && r.contains(p)
                {
                    let kind = if p.x - r.left() < EDGE_PX {
                        DragKind::TrimStart
                    } else if r.right() - p.x < EDGE_PX {
                        DragKind::TrimEnd
                    } else {
                        DragKind::Move
                    };
                    hit = Some((i, c.id, kind));
                }
            }
        }

        // Cursor en los bordes.
        if self.drag.is_none()
            && bg.hovered()
            && let Some((_, _, k)) = hit
            && k != DragKind::Move
        {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }

        // Clic: seleccionar o mover el cabezal.
        if bg.clicked() || (bg.drag_started() && hit.is_none()) {
            out.focused = true;
            match (hit, pointer) {
                (Some((i, id, _)), _) if bg.clicked() => self.selected = Some((i, id)),
                (_, Some(p)) => {
                    self.selected = None;
                    out.seek = Some(frame_of(p.x).max(0));
                }
                _ => {}
            }
        }
        // Arrastrar un clip lo mueve o recorta; arrastrar en la regla o en un hueco, el cabezal.
        if bg.drag_started()
            && let Some((i, id, kind)) = hit
        {
            out.focused = true;
            self.selected = Some((i, id));
            self.drag = Some(Drag { track: i, id, kind, delta: 0, target: i });
        }
        if bg.dragged()
            && self.drag.is_none()
            && let Some(p) = pointer
        {
            out.seek = Some(frame_of(p.x).max(0));
        }
        if let Some(mut d) = self.drag {
            // Al soltar ya no hay punto de origen: se queda el desplazamiento del último fotograma.
            let origin = ui.input(|i| i.pointer.press_origin());
            if let (Some(o), Some(p)) = (origin, pointer) {
                let raw_delta = ((p.x - o.x) / self.zoom).round() as Frames;
                d.delta = self.snap(project, playhead, d, raw_delta);
            }
            if d.kind == DragKind::Move
                && let Some(p) = pointer
                && let Some(t) = row_at(p)
                && project.timeline.tracks[t].kind == project.timeline.tracks[d.track].kind
            {
                d.target = t;
            }
            self.drag = Some(d);
            // Fantasma de dónde caería.
            if let Some(c) = project.timeline.tracks[d.track].clip(d.id)
                && let Some(row) = row_rect(d.target)
            {
                let (s, e) = match d.kind {
                    DragKind::Move => (c.start + d.delta, c.end() + d.delta),
                    DragKind::TrimStart => (c.start + d.delta, c.end()),
                    DragKind::TrimEnd => (c.start, c.end() + d.delta),
                };
                let ghost =
                    Rect::from_min_max(Pos2::new(x_of(s), row.top() + 3.0), Pos2::new(x_of(e), row.bottom() - 3.0));
                painter.rect_stroke(ghost, 3.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Inside);
            }
            if bg.drag_stopped() {
                self.drag = None;
                match self.finish_drag(project, d) {
                    Ok(edit) => out.edit = edit,
                    Err(e) => out.error = Some(e),
                }
                if d.kind == DragKind::Move && d.target != d.track {
                    self.selected = Some((d.target, d.id));
                }
            }
        }

        // Cabezal.
        let x = x_of(playhead);
        if x >= lane_left && x <= rect.right() {
            painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], Stroke::new(1.5, ACCENT));
            let tip = [Pos2::new(x - 6.0, rect.top()), Pos2::new(x + 6.0, rect.top()), Pos2::new(x, rect.top() + 8.0)];
            painter.add(egui::Shape::convex_polygon(tip.to_vec(), ACCENT, Stroke::NONE));
        }
        if project.timeline.duration() == 0 {
            painter.text(
                Pos2::new(lane_left + 16.0, rect.bottom() - 18.0),
                Align2::LEFT_CENTER,
                "Marca un tramo en el visor de origen (I / O) y pulsa F9 para insertarlo, F10 para sobrescribir o Mayús+F12 para añadirlo al final.",
                FontId::proportional(13.0),
                Color32::GRAY,
            );
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_clip(
        &self,
        painter: &egui::Painter,
        project: &Project,
        c: &Clip,
        kind: TrackKind,
        r: Rect,
        selected: bool,
        dragging: bool,
    ) {
        let fill = match kind {
            TrackKind::Video => Color32::from_rgb(0x3A, 0x5A, 0x8C),
            TrackKind::Audio => Color32::from_rgb(0x3C, 0x7A, 0x5A),
        };
        let fill = if dragging { fill.gamma_multiply(0.5) } else { fill };
        painter.rect_filled(r, 3.0, fill);
        let stroke = if selected { Stroke::new(2.0, ACCENT) } else { Stroke::new(1.0, Color32::from_black_alpha(120)) };
        painter.rect_stroke(r, 3.0, stroke, StrokeKind::Inside);
        if r.width() > 24.0 {
            let name = project.media.iter().find(|m| m.id == c.media).map_or("¿medio?", |m| m.name.as_str());
            let clipped = painter.with_clip_rect(r.shrink(2.0).intersect(painter.clip_rect()));
            clipped.text(
                r.left_top() + Vec2::new(6.0, 4.0),
                Align2::LEFT_TOP,
                name,
                FontId::proportional(12.0),
                Color32::WHITE,
            );
        }
    }

    /// Ajusta el desplazamiento para que el borde que se arrastra se pegue al cabezal o a los
    /// bordes de otros clips si está cerca.
    fn snap(&self, project: &Project, playhead: Frames, d: Drag, raw: Frames) -> Frames {
        let Some(c) = project.timeline.tracks.get(d.track).and_then(|t| t.clip(d.id)) else { return raw };
        let mut points = vec![playhead, 0];
        for t in &project.timeline.tracks {
            for o in t.clips().iter().filter(|o| o.id != d.id) {
                points.push(o.start);
                points.push(o.end());
            }
        }
        let edges: Vec<Frames> = match d.kind {
            DragKind::Move => vec![c.start, c.end()],
            DragKind::TrimStart => vec![c.start],
            DragKind::TrimEnd => vec![c.end()],
        };
        let limit = (SNAP_PX / self.zoom).ceil() as Frames;
        let mut best: Option<(Frames, Frames)> = None;
        for e in edges {
            for &p in &points {
                let adj = p - e;
                let dist = (adj - raw).abs();
                if dist <= limit && best.is_none_or(|(bd, _)| dist < bd) {
                    best = Some((dist, adj));
                }
            }
        }
        best.map_or(raw, |(_, adj)| adj)
    }

    fn finish_drag(&self, project: &Project, d: Drag) -> Result<Option<Edit>, cinder_core::EditError> {
        if d.delta == 0 && d.target == d.track {
            return Ok(None);
        }
        let track = project.timeline.track(d.track)?;
        let c = track.clip(d.id).ok_or(cinder_core::EditError::NoSuchClip(d.id))?;
        let delta = d.delta;
        let edit = match d.kind {
            // Mover a otra pista: solo ese clip.
            DragKind::Move if d.target != d.track => Edit::Batch(vec![
                Edit::RemoveClip { track: d.track, id: d.id },
                Edit::AddClip { track: d.target, clip: Clip { start: c.start + delta, ..c.clone() } },
            ]),
            // En la misma pista se mueve o recorta junto con su pareja.
            DragKind::Move => ops::on_linked(project, d.track, d.id, |p, t, id| {
                let o = p.timeline.track(t)?.clip(id).ok_or(cinder_core::EditError::NoSuchClip(id))?;
                Ok(Edit::MoveClip { track: t, id, to: o.start + delta })
            })?,
            DragKind::TrimStart => ops::on_linked(project, d.track, d.id, |p, t, id| ops::trim_start(p, t, id, delta))?,
            DragKind::TrimEnd => ops::on_linked(project, d.track, d.id, |p, t, id| ops::trim_end(p, t, id, delta))?,
        };
        Ok(Some(edit))
    }
}

/// Separación entre marcas de la regla: la menor de una lista que deje al menos 90 px.
fn ruler_step(zoom: f32, fps: f64) -> Frames {
    let f = fps.round().max(1.0) as Frames;
    let candidates = [1, 5, 10, f, 2 * f, 5 * f, 10 * f, 30 * f, 60 * f, 300 * f, 600 * f, 1800 * f, 3600 * f];
    candidates.into_iter().find(|&s| s as f32 * zoom >= 90.0).unwrap_or(3600 * f)
}

#[cfg(test)]
mod tests {
    use super::ruler_step;

    #[test]
    fn la_regla_no_amontona_marcas() {
        assert_eq!(ruler_step(40.0, 25.0), 5);
        assert_eq!(ruler_step(4.0, 25.0), 25);
        assert_eq!(ruler_step(0.05, 25.0), 7500);
    }
}
