//! Línea de tiempo de la página Edición: código de tiempo y regla arriba, cabeceras de pista a
//! la izquierda y los clips con su miniatura o su forma de onda.
//!
//! Ratón (herramienta de selección): clic en la regla mueve el cabezal · clic en un clip lo
//! selecciona (con Alt, sin su pareja) · arrastrar un clip lo mueve (también a otra pista del
//! mismo tipo) · arrastrar un borde lo recorta · Ctrl+rueda acerca o aleja · rueda desplaza.
//! Con la cuchilla, un clic en un clip lo corta ahí. Botón derecho: añadir o quitar pistas.
//! En la cabecera, el nombre de la pista la elige como destino y el icono la silencia u oculta.

use cinder_core::{Clip, ClipId, Edit, EditError, Frames, MediaId, Project, TrackKind, ops};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

use crate::icons::{self, Icon};
use crate::media_pool::{MediaPool, PEAKS_PER_SEC};
use crate::monitor::timecode;
use crate::theme;

pub const HEADER_W: f32 = 156.0;
const RULER_H: f32 = 30.0;
const VIDEO_H: f32 = 60.0;
const AUDIO_H: f32 = 48.0;
/// Separación entre el grupo de pistas de vídeo y el de audio.
const DIVIDER_H: f32 = 5.0;
/// Franja con el nombre en cada clip.
const NAME_H: f32 = 15.0;
/// Distancia en píxeles a la que un borde se pega a otro al arrastrar.
const SNAP_PX: f32 = 8.0;
/// Zona de cada borde del clip que recorta en vez de mover.
const EDGE_PX: f32 = 6.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Blade,
}

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

/// Lo que pide la línea de tiempo tras un repintado.
pub enum Action {
    Seek(Frames),
    Apply(Edit),
    Error(EditError),
    /// Se tocó la línea de tiempo: el visor de programa pasa a ser el activo.
    Focus,
    /// Cuchilla sobre un clip.
    Blade {
        track: usize,
        id: ClipId,
        at: Frames,
    },
    AddTrack(TrackKind),
    RemoveTrack(usize),
    ToggleMute(usize),
    /// Pista elegida como destino de F9/F10/Mayús+F12.
    SetDest(usize),
    /// Un clip del panel de medios soltado aquí.
    DropMedia {
        media: MediaId,
        at: Frames,
    },
}

/// Lo que la línea de tiempo necesita ver para dibujarse.
pub struct View<'a> {
    pub project: &'a Project,
    pub pool: &'a MediaPool,
    pub playhead: Frames,
    pub playing: bool,
    pub dest_video: usize,
    pub dest_audio: usize,
}

pub struct TimelineView {
    /// Píxeles por fotograma.
    pub zoom: f32,
    /// Primer fotograma visible.
    scroll: f32,
    pub selected: Option<(usize, ClipId)>,
    /// La selección se hizo con Alt: solo ese clip, sin su pareja.
    alone: bool,
    pub tool: Tool,
    /// Imán: al arrastrar, los bordes se pegan al cabezal y a otros clips.
    pub snapping: bool,
    /// Selección enlazada: el vídeo y el audio de una toma van juntos.
    pub linked: bool,
    drag: Option<Drag>,
    /// Se está arrastrando el cabezal por la regla.
    scrubbing: bool,
    /// Pista sobre la que se abrió el menú contextual.
    menu_track: Option<usize>,
    /// Ancho del último dibujado, para encajar.
    width: f32,
    last_playhead: Frames,
}

impl TimelineView {
    pub fn new() -> Self {
        Self {
            zoom: 4.0,
            scroll: 0.0,
            selected: None,
            alone: false,
            tool: Tool::Select,
            snapping: true,
            linked: true,
            drag: None,
            scrubbing: false,
            menu_track: None,
            width: 900.0,
            last_playhead: 0,
        }
    }

    pub fn zoom_by(&mut self, factor: f32) {
        self.zoom = (self.zoom * factor).clamp(0.02, 40.0);
    }

    /// Encaja la línea de tiempo entera en el ancho visible.
    pub fn fit(&mut self, duration: Frames) {
        if duration > 0 {
            self.zoom = ((self.width - HEADER_W - 30.0) / duration as f32).clamp(0.02, 40.0);
            self.scroll = 0.0;
        }
    }

    /// El clip seleccionado y, con la selección enlazada, sus parejas.
    pub fn selection(&self, project: &Project) -> Vec<(usize, ClipId)> {
        match self.selected {
            Some((t, id)) if self.linked && !self.alone => ops::linked(project, t, id),
            Some((t, id)) if project.timeline.tracks.get(t).is_some_and(|tr| tr.clip(id).is_some()) => vec![(t, id)],
            _ => Vec::new(),
        }
    }

    /// La misma operación sobre toda la selección, como un solo cambio.
    pub fn on_selection(
        &self,
        project: &Project,
        op: impl Fn(&Project, usize, ClipId) -> Result<Edit, EditError>,
    ) -> Result<Edit, EditError> {
        let edits =
            self.selection(project).into_iter().map(|(t, id)| op(project, t, id)).collect::<Result<Vec<_>, _>>()?;
        if edits.is_empty() {
            return Err(EditError::NothingThere);
        }
        Ok(Edit::Batch(edits))
    }

    /// Orden en pantalla: vídeo de arriba abajo (V2 sobre V1) y luego audio (A1, A2…).
    fn row_order(project: &Project) -> (Vec<usize>, Vec<usize>) {
        let tracks = &project.timeline.tracks;
        let mut video: Vec<usize> = (0..tracks.len()).filter(|&i| tracks[i].kind == TrackKind::Video).collect();
        video.reverse();
        let audio = (0..tracks.len()).filter(|&i| tracks[i].kind == TrackKind::Audio).collect();
        (video, audio)
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, v: &View<'_>) -> Vec<Action> {
        let mut out = Vec::new();
        let project = v.project;
        let fps = project.settings.fps.as_f64();
        let rect = ui.available_rect_before_wrap();
        self.width = rect.width();
        let bg = ui.allocate_rect(rect, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme::BG);
        let lane_left = rect.left() + HEADER_W;
        let lanes_width = rect.width() - HEADER_W;
        let lanes = Rect::from_min_max(Pos2::new(lane_left, rect.top()), rect.max);
        let ruler = Rect::from_min_size(lanes.min, Vec2::new(lanes.width(), RULER_H));

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
        }
        // Si el cabezal se mueve y se sale de la vista (reproduciendo, o tras insertar), la
        // vista salta a él.
        if v.playhead != self.last_playhead {
            self.last_playhead = v.playhead;
            let x = lane_left + (v.playhead as f32 - self.scroll) * self.zoom;
            if x > rect.right() - 24.0 {
                self.scroll = v.playhead as f32 - if v.playing { 60.0 } else { lanes_width * 0.6 } / self.zoom;
            } else if x < lane_left {
                self.scroll = v.playhead as f32 - 60.0 / self.zoom;
            }
        }
        self.scroll = self.scroll.max(0.0);
        let (scroll, zoom) = (self.scroll, self.zoom);
        let x_of = |f: Frames| lane_left + (f as f32 - scroll) * zoom;
        let frame_of = |x: f32| ((x - lane_left) / zoom + scroll).round() as Frames;

        // Filas.
        let (video, audio) = Self::row_order(project);
        let mut rows: Vec<(usize, Rect)> = Vec::new();
        let mut y = ruler.bottom() + 1.0;
        for &i in &video {
            rows.push((i, Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), VIDEO_H))));
            y += VIDEO_H + 1.0;
        }
        let divider = Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), DIVIDER_H));
        y += DIVIDER_H + 1.0;
        for &i in &audio {
            rows.push((i, Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), AUDIO_H))));
            y += AUDIO_H + 1.0;
        }
        let row_rect = |track: usize| rows.iter().find(|(i, _)| *i == track).map(|(_, r)| *r);
        let row_at = |p: Pos2| rows.iter().find(|(_, r)| r.contains(p)).map(|(i, _)| *i);

        // Columna de cabeceras y esquina con el código de tiempo.
        painter.rect_filled(Rect::from_min_max(rect.min, Pos2::new(lane_left, rect.bottom())), 0.0, theme::PANEL);
        painter.text(
            Pos2::new(rect.left() + 12.0, ruler.center().y),
            Align2::LEFT_CENTER,
            timecode(v.playhead as f64 / fps, fps),
            FontId::monospace(17.0),
            theme::TEXT_BRIGHT,
        );
        painter.rect_filled(divider, 0.0, theme::LINE);

        // Regla.
        painter.rect_filled(ruler, 0.0, theme::PANEL);
        painter.line_segment([ruler.left_bottom(), ruler.right_bottom()], Stroke::new(1.0, theme::LINE));
        let lane_painter = painter.with_clip_rect(lanes);
        let step = ruler_step(zoom, fps);
        let minor = (step / 5).max(1);
        let mut f = (scroll as Frames / step) * step;
        while x_of(f) < rect.right() {
            let x = x_of(f);
            lane_painter.line_segment(
                [Pos2::new(x, ruler.bottom() - 9.0), Pos2::new(x, ruler.bottom())],
                Stroke::new(1.0, theme::TEXT_DIM),
            );
            lane_painter.text(
                Pos2::new(x + 4.0, ruler.top() + 5.0),
                Align2::LEFT_TOP,
                timecode(f as f64 / fps, fps),
                FontId::monospace(10.0),
                theme::TEXT_DIM,
            );
            if minor * 5 == step && minor as f32 * zoom >= 6.0 {
                for k in 1..5 {
                    let xm = x_of(f + k * minor);
                    lane_painter.line_segment(
                        [Pos2::new(xm, ruler.bottom() - 4.0), Pos2::new(xm, ruler.bottom())],
                        Stroke::new(1.0, theme::RAISED_HOVER),
                    );
                }
            }
            f += step;
        }

        // Carriles y cabeceras.
        for &(i, row) in &rows {
            let t = &project.timeline.tracks[i];
            let lane = Rect::from_min_max(Pos2::new(lane_left, row.top()), row.max);
            painter.rect_filled(lane, 0.0, theme::LANE);
            let head = Rect::from_min_max(row.min, Pos2::new(lane_left - 1.0, row.bottom()));
            painter.rect_filled(head, 0.0, theme::RAISED);

            let is_dest = i == v.dest_video || i == v.dest_audio;
            let badge =
                Rect::from_min_size(Pos2::new(head.left() + 8.0, head.center().y - 10.0), Vec2::new(32.0, 20.0));
            let badge_resp = ui.allocate_rect(badge, Sense::click());
            painter.rect_filled(badge, 3.0, if badge_resp.hovered() { theme::RAISED_HOVER } else { theme::PANEL });
            if is_dest {
                painter.rect_stroke(badge, 3.0, Stroke::new(1.5, theme::HOT), StrokeKind::Inside);
            }
            painter.text(
                badge.center(),
                Align2::CENTER_CENTER,
                &t.name,
                FontId::proportional(11.5),
                if is_dest { theme::TEXT_BRIGHT } else { theme::TEXT },
            );
            if badge_resp.on_hover_text("Pista de destino para insertar, sobrescribir y añadir").clicked() {
                out.push(Action::SetDest(i));
            }
            let n = t.clips().len();
            let kind = if t.kind == TrackKind::Video { "Vídeo" } else { "Audio" };
            painter.text(
                Pos2::new(badge.right() + 8.0, head.center().y),
                Align2::LEFT_CENTER,
                format!("{kind} · {n}"),
                FontId::proportional(10.5),
                theme::TEXT_DIM,
            );
            let (icon, tip) = match (t.kind, t.muted) {
                (TrackKind::Video, false) => (Icon::Eye, "Ocultar la pista"),
                (TrackKind::Video, true) => (Icon::Eye, "Mostrar la pista"),
                (TrackKind::Audio, false) => (Icon::Speaker, "Silenciar la pista"),
                (TrackKind::Audio, true) => (Icon::Speaker, "Quitar el silencio"),
            };
            let mute = Rect::from_center_size(Pos2::new(head.right() - 18.0, head.center().y), Vec2::splat(24.0));
            if icons::button_at(ui, mute, icon, false, t.muted).on_hover_text(tip).clicked() {
                out.push(Action::ToggleMute(i));
            }
        }

        let pointer = ui.input(|i| i.pointer.interact_pos());
        // Al empezar un arrastre el ratón ya se ha movido unos píxeles: lo que cuenta es dónde
        // se pulsó (si no, agarrar un borde acabaría moviendo el clip).
        let probe = if bg.drag_started() { ui.input(|i| i.pointer.press_origin()).or(pointer) } else { pointer };
        let in_lanes = |p: Pos2| p.x >= lane_left && p.y > ruler.bottom();
        let in_ruler = |p: Pos2| p.x >= lane_left && p.y <= ruler.bottom();

        // Clips (y qué hay bajo el ratón).
        let selection = self.selection(project);
        let mut hit: Option<(usize, ClipId, DragKind)> = None;
        for &(i, row) in &rows {
            let t = &project.timeline.tracks[i];
            for c in t.clips() {
                let r = Rect::from_min_max(
                    Pos2::new(x_of(c.start), row.top() + 2.0),
                    Pos2::new(x_of(c.end()), row.bottom() - 2.0),
                );
                if r.right() < lane_left || r.left() > rect.right() {
                    continue;
                }
                let dragging = self.drag.is_some_and(|d| d.id == c.id);
                let selected = selection.contains(&(i, c.id));
                paint_clip(&lane_painter, v, c, t.kind, r, lanes, selected, dragging || t.muted, zoom, fps);
                if let Some(p) = probe
                    && in_lanes(p)
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

        let alt = ui.input(|i| i.modifiers.alt);
        match self.tool {
            Tool::Select => {
                if self.drag.is_none()
                    && bg.hovered()
                    && let Some((_, _, k)) = hit
                    && k != DragKind::Move
                {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
                if bg.clicked()
                    && let Some(p) = pointer
                {
                    out.push(Action::Focus);
                    if in_ruler(p) {
                        out.push(Action::Seek(frame_of(p.x).max(0)));
                    } else if in_lanes(p) {
                        self.selected = hit.map(|(i, id, _)| (i, id));
                        self.alone = alt;
                    }
                }
                if bg.drag_started()
                    && let Some(p) = probe
                {
                    out.push(Action::Focus);
                    if in_ruler(p) {
                        self.scrubbing = true;
                    } else if let Some((i, id, kind)) = hit {
                        self.selected = Some((i, id));
                        self.alone = alt;
                        self.drag = Some(Drag { track: i, id, kind, delta: 0, target: i });
                    }
                }
            }
            Tool::Blade => {
                if bg.hovered()
                    && let Some(p) = pointer
                    && in_lanes(p)
                {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                    let mut at = frame_of(p.x);
                    if self.snapping && ((at - v.playhead).abs() as f32) * zoom <= SNAP_PX {
                        at = v.playhead;
                    }
                    let x = x_of(at);
                    painter.line_segment(
                        [Pos2::new(x, ruler.bottom()), Pos2::new(x, rect.bottom())],
                        Stroke::new(1.0, theme::TEXT_BRIGHT),
                    );
                    if bg.clicked()
                        && let Some((track, id, _)) = hit
                    {
                        out.push(Action::Focus);
                        out.push(Action::Blade { track, id, at });
                    }
                }
                if (bg.clicked() || bg.drag_started())
                    && let Some(p) = probe
                    && in_ruler(p)
                {
                    out.push(Action::Focus);
                    out.push(Action::Seek(frame_of(p.x).max(0)));
                    self.scrubbing = bg.drag_started();
                }
            }
        }
        if self.scrubbing {
            if let Some(p) = pointer {
                out.push(Action::Seek(frame_of(p.x).max(0)));
            }
            if bg.drag_stopped() || !bg.dragged() {
                self.scrubbing = false;
            }
        }

        // Arrastre de un clip.
        if let Some(mut d) = self.drag {
            // Al soltar ya no hay punto de origen: se queda el desplazamiento del último fotograma.
            let origin = ui.input(|i| i.pointer.press_origin());
            if let (Some(o), Some(p)) = (origin, pointer) {
                let raw = ((p.x - o.x) / zoom).round() as Frames;
                d.delta = if self.snapping { self.snap(project, v.playhead, d, raw) } else { raw };
                if d.kind == DragKind::Move
                    && let Some(t) = row_at(p)
                    && project.timeline.tracks[t].kind == project.timeline.tracks[d.track].kind
                {
                    d.target = t;
                }
            }
            self.drag = Some(d);
            if let Some(c) = project.timeline.tracks.get(d.track).and_then(|t| t.clip(d.id))
                && let Some(row) = row_rect(d.target)
            {
                let (s, e) = match d.kind {
                    DragKind::Move => (c.start + d.delta, c.end() + d.delta),
                    DragKind::TrimStart => (c.start + d.delta, c.end()),
                    DragKind::TrimEnd => (c.start, c.end() + d.delta),
                };
                let ghost =
                    Rect::from_min_max(Pos2::new(x_of(s), row.top() + 2.0), Pos2::new(x_of(e), row.bottom() - 2.0));
                lane_painter.rect_stroke(ghost, 2.0, Stroke::new(1.5, theme::TEXT_BRIGHT), StrokeKind::Inside);
            }
            if bg.drag_stopped() || !bg.dragged() {
                self.drag = None;
                match self.finish_drag(project, d) {
                    Ok(Some(edit)) => {
                        out.push(Action::Apply(edit));
                        if d.kind == DragKind::Move && d.target != d.track {
                            self.selected = Some((d.target, d.id));
                        }
                    }
                    Ok(None) => {}
                    Err(e) => out.push(Action::Error(e)),
                }
            }
        }

        // Un clip del panel de medios soltado en la línea de tiempo.
        if let Some(p) = ui.input(|i| i.pointer.hover_pos())
            && lanes.contains(p)
            && egui::DragAndDrop::has_payload_of_type::<MediaId>(ui.ctx())
        {
            let at = self.snap_point(project, v.playhead, frame_of(p.x).max(0));
            let x = x_of(at);
            painter.line_segment(
                [Pos2::new(x, ruler.bottom()), Pos2::new(x, rect.bottom())],
                Stroke::new(2.0, theme::TEXT_BRIGHT),
            );
            if let Some(media) = bg.dnd_release_payload::<MediaId>() {
                out.push(Action::Focus);
                out.push(Action::DropMedia { media: *media, at });
            }
        }

        // Menú contextual: pistas.
        if bg.secondary_clicked() {
            self.menu_track = pointer.and_then(row_at);
        }
        let menu_track = self.menu_track;
        bg.context_menu(|ui| {
            if ui.button("Añadir pista de vídeo").clicked() {
                out.push(Action::AddTrack(TrackKind::Video));
                ui.close();
            }
            if ui.button("Añadir pista de audio").clicked() {
                out.push(Action::AddTrack(TrackKind::Audio));
                ui.close();
            }
            if let Some(t) = menu_track
                && let Some(track) = project.timeline.tracks.get(t)
            {
                ui.separator();
                if ui.button(format!("Eliminar la pista {}", track.name)).clicked() {
                    out.push(Action::RemoveTrack(t));
                    ui.close();
                }
            }
        });

        // Cabezal.
        let x = x_of(v.playhead);
        if x >= lane_left - 1.0 && x <= rect.right() {
            painter.line_segment(
                [Pos2::new(x, ruler.top() + 8.0), Pos2::new(x, rect.bottom())],
                Stroke::new(1.5, theme::HOT),
            );
            let top = ruler.top() + 2.0;
            let head = vec![
                Pos2::new(x - 6.0, top),
                Pos2::new(x + 6.0, top),
                Pos2::new(x + 6.0, top + 8.0),
                Pos2::new(x, top + 14.0),
                Pos2::new(x - 6.0, top + 8.0),
            ];
            painter.add(egui::Shape::convex_polygon(head, theme::HOT, Stroke::NONE));
        }
        if project.timeline.duration() == 0 {
            painter.text(
                Pos2::new(lane_left + 18.0, (y + 22.0).min(rect.bottom() - 12.0)),
                Align2::LEFT_CENTER,
                "Arrastra un clip desde Medios, o marca un tramo en el visor de origen (I / O) y pulsa F9, F10 o Mayús+F12.",
                FontId::proportional(12.0),
                theme::TEXT_DIM,
            );
        }
        out
    }

    /// Con el imán, el punto más cercano entre el cabezal, el principio y los bordes de los
    /// clips, si está a pocos píxeles; si no, el mismo punto.
    fn snap_point(&self, project: &Project, playhead: Frames, at: Frames) -> Frames {
        if !self.snapping {
            return at;
        }
        let limit = (SNAP_PX / self.zoom).ceil() as Frames;
        let clips = project.timeline.tracks.iter().flat_map(|t| t.clips().iter().flat_map(|c| [c.start, c.end()]));
        [playhead, 0]
            .into_iter()
            .chain(clips)
            .filter(|p| (p - at).abs() <= limit)
            .min_by_key(|p| (p - at).abs())
            .unwrap_or(at)
    }

    /// Ajusta el desplazamiento para que el borde que se arrastra se pegue al cabezal o a los
    /// bordes de otros clips si está cerca.
    fn snap(&self, project: &Project, playhead: Frames, d: Drag, raw: Frames) -> Frames {
        let Some(c) = project.timeline.tracks.get(d.track).and_then(|t| t.clip(d.id)) else { return raw };
        let moving = self.selection(project);
        let mut points = vec![playhead, 0];
        for (i, t) in project.timeline.tracks.iter().enumerate() {
            for o in t.clips().iter().filter(|o| !moving.contains(&(i, o.id))) {
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

    fn finish_drag(&self, project: &Project, d: Drag) -> Result<Option<Edit>, EditError> {
        if d.delta == 0 && d.target == d.track {
            return Ok(None);
        }
        let c = project.timeline.track(d.track)?.clip(d.id).ok_or(EditError::NoSuchClip(d.id))?;
        let delta = d.delta;
        let edit = match d.kind {
            // A otra pista: solo ese clip.
            DragKind::Move if d.target != d.track => Edit::Batch(vec![
                Edit::RemoveClip { track: d.track, id: d.id },
                Edit::AddClip { track: d.target, clip: Clip { start: c.start + delta, ..c.clone() } },
            ]),
            DragKind::Move => self.on_selection(project, |p, t, id| {
                let o = p.timeline.track(t)?.clip(id).ok_or(EditError::NoSuchClip(id))?;
                Ok(Edit::MoveClip { track: t, id, to: o.start + delta })
            })?,
            DragKind::TrimStart => self.on_selection(project, |p, t, id| ops::trim_start(p, t, id, delta))?,
            DragKind::TrimEnd => self.on_selection(project, |p, t, id| ops::trim_end(p, t, id, delta))?,
        };
        Ok(Some(edit))
    }
}

/// Dibuja un clip: cuerpo, miniatura o forma de onda, franja con el nombre y contorno.
#[allow(clippy::too_many_arguments)]
fn paint_clip(
    painter: &egui::Painter,
    v: &View<'_>,
    c: &Clip,
    kind: TrackKind,
    r: Rect,
    lanes: Rect,
    selected: bool,
    dim: bool,
    zoom: f32,
    fps: f64,
) {
    let (body_color, strip_color) = match kind {
        TrackKind::Video => (theme::CLIP_VIDEO, theme::CLIP_VIDEO_DARK),
        TrackKind::Audio => (theme::CLIP_AUDIO, theme::CLIP_AUDIO_DARK),
    };
    let fade = |col: Color32| if dim { col.gamma_multiply(0.45) } else { col };
    painter.rect_filled(r, 2.0, fade(body_color));
    let visible = r.intersect(lanes);
    let name = v.project.media.iter().find(|m| m.id == c.media).map_or("¿medio?", |m| m.name.as_str());
    match kind {
        TrackKind::Video => {
            let body = Rect::from_min_max(r.min, Pos2::new(r.right(), r.bottom() - NAME_H));
            if let Some(tex) = v.pool.thumb(c.media) {
                let size = tex.size_vec2();
                let w = body.height() * size.x / size.y.max(1.0);
                if r.width() > w + 12.0 {
                    let img = Rect::from_min_size(body.min + Vec2::new(1.0, 1.0), Vec2::new(w, body.height() - 1.0));
                    let uv = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));
                    painter.image(tex.id(), img, uv, fade(Color32::WHITE));
                }
            }
            let strip = Rect::from_min_max(Pos2::new(r.left(), r.bottom() - NAME_H), r.max);
            painter.rect_filled(strip, 0.0, fade(strip_color));
            if visible.width() > 24.0 {
                painter.with_clip_rect(visible.shrink(2.0)).text(
                    Pos2::new(visible.left() + 5.0, strip.center().y),
                    Align2::LEFT_CENTER,
                    name,
                    FontId::proportional(10.5),
                    fade(theme::TEXT_BRIGHT),
                );
            }
        }
        TrackKind::Audio => {
            let strip = Rect::from_min_size(r.min, Vec2::new(r.width(), NAME_H - 2.0));
            painter.rect_filled(strip, 0.0, fade(strip_color));
            let body = Rect::from_min_max(Pos2::new(r.left(), strip.bottom()), r.max);
            if let Some(peaks) = v.pool.peaks(c.media) {
                paint_wave(painter, peaks, c, r.left(), body, visible, zoom, fps, fade(theme::WAVE));
            }
            if visible.width() > 24.0 {
                painter.with_clip_rect(visible.shrink(2.0)).text(
                    Pos2::new(visible.left() + 5.0, strip.center().y),
                    Align2::LEFT_CENTER,
                    name,
                    FontId::proportional(10.5),
                    fade(theme::TEXT_BRIGHT),
                );
            }
        }
    }
    let stroke = if selected { Stroke::new(2.0, theme::HOT) } else { Stroke::new(1.0, theme::LINE) };
    painter.rect_stroke(r, 2.0, stroke, StrokeKind::Inside);
}

/// Forma de onda del tramo del medio que usa el clip, una línea por columna de píxeles.
#[allow(clippy::too_many_arguments)]
fn paint_wave(
    painter: &egui::Painter,
    peaks: &[f32],
    c: &Clip,
    clip_left: f32,
    body: Rect,
    visible: Rect,
    zoom: f32,
    fps: f64,
    color: Color32,
) {
    let mid = body.center().y;
    let half = body.height() * 0.46;
    let per_px = f64::from(PEAKS_PER_SEC) / (f64::from(zoom) * fps);
    let mut x = visible.left().floor();
    while x < visible.right() {
        let frames = c.source_in as f64 + f64::from(x - clip_left) / f64::from(zoom);
        let from = (frames / fps * f64::from(PEAKS_PER_SEC)) as usize;
        let to = (from + (per_px.ceil() as usize).clamp(1, 512)).min(peaks.len());
        let amp = peaks.get(from..to).map_or(0.0, |s| s.iter().copied().fold(0.0f32, f32::max));
        // Escala comprimida para que el audio flojo también se vea.
        let h = (amp.powf(0.6) * half).max(0.5);
        painter.line_segment([Pos2::new(x + 0.5, mid - h), Pos2::new(x + 0.5, mid + h)], Stroke::new(1.0, color));
        x += 1.0;
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
