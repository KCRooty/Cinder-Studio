//! Panel de medios: carpetas (bins) a la izquierda y los clips en miniaturas a la derecha.
//! Un clic abre el clip en el visor de origen; se puede arrastrar a la línea de tiempo.
//!
//! Las miniaturas y las formas de onda se calculan en un hilo aparte, de una en una, para que
//! importar muchos archivos no congele la interfaz.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use cinder_core::{Bin, Frames, MediaId, MediaItem, Project, Rational};
use cinder_media::{MediaInfo, VideoFrame};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, TextureHandle, Vec2};

use crate::icons::{self, Icon};
use crate::monitor::timecode;
use crate::theme;

/// Tramos por segundo de las formas de onda.
pub const PEAKS_PER_SEC: u32 = 100;
const THUMB_W: f32 = 120.0;
const THUMB_H: f32 = 68.0;
const CELL_H: f32 = THUMB_H + 34.0;

/// Fotogramas por segundo como fracción exacta: los NTSC (23,976, 29,97, 59,94) se reconocen.
pub fn fps_rational(fps: f64) -> Rational {
    for base in [24u32, 30, 48, 60, 120] {
        let ntsc = f64::from(base) * 1000.0 / 1001.0;
        if (fps - ntsc).abs() < 0.005 {
            return Rational::new(base * 1000, 1001);
        }
    }
    let whole = fps.round();
    if (1.0..=1000.0).contains(&whole) { Rational::new(whole as u32, 1) } else { Rational::new(30, 1) }
}

pub fn secs_to_frames(secs: f64, fps: Rational) -> Frames {
    (secs * fps.as_f64()).round() as Frames
}

pub fn frames_to_secs(frames: Frames, fps: Rational) -> f64 {
    frames as f64 / fps.as_f64()
}

enum Asset {
    Thumb(MediaId, VideoFrame),
    Peaks(MediaId, Vec<f32>),
}

/// Lo que pide el panel tras un repintado.
#[derive(Default)]
pub struct PoolOutput {
    /// Clip en el que se hizo clic: abrirlo en el visor de origen.
    pub open: Option<MediaId>,
    /// Se pulsó importar.
    pub import: bool,
    /// Cambió algo del proyecto (carpetas, medios).
    pub modified: bool,
    /// Se quitó un medio: puede que estuviera abierto en algún visor.
    pub removed: Option<MediaId>,
    pub error: Option<String>,
}

pub struct MediaPool {
    infos: HashMap<MediaId, MediaInfo>,
    thumbs: HashMap<MediaId, TextureHandle>,
    peaks: HashMap<MediaId, Arc<Vec<f32>>>,
    jobs: Sender<(MediaId, PathBuf)>,
    assets: Receiver<Asset>,
    errors: Vec<String>,
    pub selected: Option<MediaId>,
    /// Carpeta que se está viendo (`None` = la principal, con todo lo que no tiene carpeta).
    bin: Option<u64>,
    /// Carpeta que se está renombrando y su texto.
    renaming: Option<(u64, String)>,
}

impl Default for MediaPool {
    fn default() -> Self {
        let (jobs, job_rx) = channel::<(MediaId, PathBuf)>();
        let (asset_tx, assets) = channel();
        // Si el hilo no arranca no hay miniaturas ni formas de onda, pero todo lo demás funciona.
        std::thread::Builder::new()
            .name("cinder-miniaturas".into())
            .spawn(move || {
                for (id, path) in job_rx {
                    if let Ok(frame) = cinder_media::thumbnail(&path, 256)
                        && asset_tx.send(Asset::Thumb(id, frame)).is_err()
                    {
                        return;
                    }
                    if let Ok(peaks) = cinder_media::peaks(&path, PEAKS_PER_SEC)
                        && asset_tx.send(Asset::Peaks(id, peaks)).is_err()
                    {
                        return;
                    }
                }
            })
            .ok();
        Self {
            infos: HashMap::new(),
            thumbs: HashMap::new(),
            peaks: HashMap::new(),
            jobs,
            assets,
            errors: Vec::new(),
            selected: None,
            bin: None,
            renaming: None,
        }
    }
}

impl MediaPool {
    pub fn info(&self, id: MediaId) -> Option<&MediaInfo> {
        self.infos.get(&id)
    }

    pub fn thumb(&self, id: MediaId) -> Option<&TextureHandle> {
        self.thumbs.get(&id)
    }

    pub fn peaks(&self, id: MediaId) -> Option<&Arc<Vec<f32>>> {
        self.peaks.get(&id)
    }

    pub fn id_of(&self, project: &Project, path: &Path) -> Option<MediaId> {
        project.media.iter().find(|m| m.path == path).map(|m| m.id)
    }

    /// Importa los archivos a la carpeta abierta y devuelve el último que se pudo abrir.
    pub fn import(&mut self, project: &mut Project, paths: &[PathBuf]) -> Option<MediaId> {
        let mut last = None;
        for path in paths {
            if let Some(id) = self.id_of(project, path) {
                last = Some(id);
                continue;
            }
            match MediaInfo::probe(path) {
                Ok(info) => {
                    // El primer clip de un proyecto vacío fija su formato.
                    if project.media.is_empty() && project.timeline.duration() == 0 && info.fps > 0.0 {
                        project.settings.fps = fps_rational(info.fps);
                        project.settings.width = info.width;
                        project.settings.height = info.height;
                    }
                    let id = MediaId(project.alloc_id());
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let duration = secs_to_frames(info.duration, project.settings.fps).max(1);
                    project.media.push(MediaItem { id, name, path: path.clone(), duration, bin: self.bin });
                    self.infos.insert(id, info);
                    self.jobs.send((id, path.clone())).ok();
                    last = Some(id);
                }
                Err(e) => self.errors.push(e.to_string()),
            }
        }
        if last.is_some() {
            self.selected = last;
        }
        last
    }

    /// Vuelve a leer la ficha de cada medio (al abrir un proyecto). Los que faltan se avisan.
    pub fn reload(&mut self, project: &Project) {
        self.infos.clear();
        self.thumbs.clear();
        self.peaks.clear();
        self.errors.clear();
        self.selected = None;
        self.bin = None;
        for m in &project.media {
            match MediaInfo::probe(&m.path) {
                Ok(info) => {
                    self.infos.insert(m.id, info);
                    self.jobs.send((m.id, m.path.clone())).ok();
                }
                Err(e) => self.errors.push(format!("{}: {e}", m.name)),
            }
        }
    }

    /// Recoge las miniaturas y formas de onda que hayan terminado.
    fn collect(&mut self, ctx: &egui::Context) {
        let mut got = false;
        while let Ok(asset) = self.assets.try_recv() {
            got = true;
            match asset {
                Asset::Thumb(id, frame) => {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [frame.width as usize, frame.height as usize],
                        &frame.rgba,
                    );
                    let tex = ctx.load_texture(format!("mini-{}", id.0), image, egui::TextureOptions::LINEAR);
                    self.thumbs.insert(id, tex);
                }
                Asset::Peaks(id, peaks) => {
                    self.peaks.insert(id, Arc::new(peaks));
                }
            }
        }
        if got {
            ctx.request_repaint();
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, project: &mut Project) -> PoolOutput {
        self.collect(ui.ctx());
        let mut out = PoolOutput::default();
        if self.bin.is_some_and(|b| !project.bins.iter().any(|x| x.id == b)) {
            self.bin = None;
        }

        // Cabecera.
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Medios").color(theme::TEXT_BRIGHT));
            let count = project.media.iter().filter(|m| m.bin == self.bin).count();
            ui.label(egui::RichText::new(format!("{count}")).color(theme::TEXT_DIM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(6.0);
                if icons::button(ui, Icon::Plus, false).on_hover_text("Importar clips (Ctrl+I)").clicked() {
                    out.import = true;
                }
                if icons::button(ui, Icon::Folder, false).on_hover_text("Carpeta nueva").clicked() {
                    let id = project.alloc_id();
                    let name = format!("Carpeta {}", project.bins.len() + 1);
                    project.bins.push(Bin { id, name: name.clone() });
                    self.bin = Some(id);
                    self.renaming = Some((id, name));
                    out.modified = true;
                }
            });
        });
        ui.add_space(2.0);
        ui.separator();

        let full = ui.available_rect_before_wrap();
        let bins_w = 118.0f32.min(full.width() * 0.4);
        let bins_rect = Rect::from_min_size(full.min, Vec2::new(bins_w, full.height()));
        let grid_rect = Rect::from_min_max(Pos2::new(full.left() + bins_w + 1.0, full.top()), full.max);
        ui.painter().rect_filled(bins_rect, 0.0, theme::LANE);
        ui.scope_builder(egui::UiBuilder::new().max_rect(bins_rect), |ui| self.bins_ui(ui, project, &mut out));
        ui.scope_builder(egui::UiBuilder::new().max_rect(grid_rect), |ui| self.grid_ui(ui, project, &mut out));
        out
    }

    fn bins_ui(&mut self, ui: &mut egui::Ui, project: &mut Project, out: &mut PoolOutput) {
        egui::ScrollArea::vertical().id_salt("bins").auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(4.0);
            let w = ui.available_width();
            if ui.add_sized([w, 20.0], egui::Button::selectable(self.bin.is_none(), "Principal")).clicked() {
                self.bin = None;
            }
            let mut delete = None;
            for bin in &mut project.bins {
                if let Some((id, text)) = self.renaming.as_mut()
                    && *id == bin.id
                {
                    let resp = ui.add_sized([w, 20.0], egui::TextEdit::singleline(text));
                    if resp.lost_focus() {
                        let name = text.trim();
                        if !name.is_empty() && name != bin.name {
                            bin.name = name.to_owned();
                            out.modified = true;
                        }
                        self.renaming = None;
                    } else if !resp.has_focus() {
                        resp.request_focus();
                    }
                    continue;
                }
                let resp = ui.add_sized([w, 20.0], egui::Button::selectable(self.bin == Some(bin.id), &bin.name));
                if resp.clicked() {
                    self.bin = Some(bin.id);
                }
                if resp.double_clicked() {
                    self.renaming = Some((bin.id, bin.name.clone()));
                }
                resp.context_menu(|ui| {
                    if ui.button("Renombrar").clicked() {
                        self.renaming = Some((bin.id, bin.name.clone()));
                        ui.close();
                    }
                    if ui.button("Eliminar carpeta").clicked() {
                        delete = Some(bin.id);
                        ui.close();
                    }
                });
            }
            if let Some(id) = delete {
                // Lo que había dentro vuelve a la principal.
                for m in project.media.iter_mut().filter(|m| m.bin == Some(id)) {
                    m.bin = None;
                }
                project.bins.retain(|b| b.id != id);
                out.modified = true;
            }
        });
    }

    fn grid_ui(&mut self, ui: &mut egui::Ui, project: &mut Project, out: &mut PoolOutput) {
        let fps = project.settings.fps.as_f64();
        let bins: Vec<(Option<u64>, String)> = std::iter::once((None, "Principal".to_owned()))
            .chain(project.bins.iter().map(|b| (Some(b.id), b.name.clone())))
            .collect();
        let mut move_to: Option<(MediaId, Option<u64>)> = None;
        let mut remove: Option<MediaId> = None;
        egui::ScrollArea::vertical().id_salt("clips").auto_shrink([false, false]).show(ui, |ui| {
            let items: Vec<&MediaItem> = project.media.iter().filter(|m| m.bin == self.bin).collect();
            if items.is_empty() {
                ui.add_space(18.0);
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new("No hay clips en esta carpeta").color(theme::TEXT_DIM));
                    ui.label(
                        egui::RichText::new("Arrastra archivos a la ventana o pulsa Ctrl+I")
                            .small()
                            .color(theme::TEXT_DIM),
                    );
                });
            }
            let avail = ui.available_width() - 8.0;
            let cols = ((avail / (THUMB_W + 10.0)).floor() as usize).max(1);
            let origin = ui.cursor().min + Vec2::new(8.0, 8.0);
            let rows = items.len().div_ceil(cols);
            let total = Vec2::new(avail, rows as f32 * (CELL_H + 8.0) + 8.0);
            let (area, _) = ui.allocate_exact_size(total, Sense::hover());
            for (n, m) in items.iter().enumerate() {
                let (col, row) = (n % cols, n / cols);
                let cell = Rect::from_min_size(
                    origin + Vec2::new(col as f32 * (THUMB_W + 10.0), row as f32 * (CELL_H + 8.0)),
                    Vec2::new(THUMB_W, CELL_H),
                );
                if !ui.is_rect_visible(cell) {
                    continue;
                }
                let resp = ui.interact(cell, ui.scope_id().with(("clip", m.id.0)), Sense::click_and_drag());
                let selected = self.selected == Some(m.id);
                let painter = ui.painter_at(area.expand(4.0));
                let thumb = Rect::from_min_size(cell.min, Vec2::new(THUMB_W, THUMB_H));
                painter.rect_filled(thumb, 2.0, theme::BG);
                match self.thumbs.get(&m.id) {
                    Some(tex) => {
                        let size = tex.size_vec2();
                        let scale = (thumb.width() / size.x).min(thumb.height() / size.y);
                        let fit = Rect::from_center_size(thumb.center(), size * scale);
                        let uv = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));
                        painter.image(tex.id(), fit, uv, Color32::WHITE);
                    }
                    None => icons::paint(
                        &painter,
                        Rect::from_center_size(thumb.center(), Vec2::splat(22.0)),
                        Icon::Film,
                        theme::TEXT_DIM,
                        false,
                    ),
                }
                let stroke = if selected {
                    Stroke::new(2.0, theme::HOT)
                } else if resp.hovered() {
                    Stroke::new(1.0, theme::TEXT_DIM)
                } else {
                    Stroke::new(1.0, theme::LINE)
                };
                painter.rect_stroke(thumb, 2.0, stroke, StrokeKind::Outside);
                let text = painter.with_clip_rect(cell.intersect(painter.clip_rect()));
                text.text(
                    Pos2::new(cell.left() + 1.0, thumb.bottom() + 5.0),
                    Align2::LEFT_TOP,
                    &m.name,
                    FontId::proportional(11.5),
                    if selected { theme::TEXT_BRIGHT } else { theme::TEXT },
                );
                let detail = match self.infos.get(&m.id) {
                    Some(_) => timecode(m.duration as f64 / fps, fps),
                    None => "no se encuentra el archivo".into(),
                };
                text.text(
                    Pos2::new(cell.left() + 1.0, thumb.bottom() + 19.0),
                    Align2::LEFT_TOP,
                    detail,
                    FontId::monospace(10.0),
                    theme::TEXT_DIM,
                );
                if resp.clicked() || resp.drag_started() {
                    self.selected = Some(m.id);
                }
                if resp.clicked() {
                    out.open = Some(m.id);
                }
                if resp.drag_started() {
                    resp.dnd_set_drag_payload(m.id);
                }
                resp.context_menu(|ui| {
                    ui.menu_button("Mover a", |ui| {
                        for (id, name) in &bins {
                            if *id != m.bin && ui.button(name).clicked() {
                                move_to = Some((m.id, *id));
                                ui.close();
                            }
                        }
                    });
                    if ui.button("Quitar del proyecto").clicked() {
                        remove = Some(m.id);
                        ui.close();
                    }
                });
            }
            for e in &self.errors {
                ui.colored_label(theme::ERROR, e);
            }
        });
        if let Some((id, bin)) = move_to
            && let Some(m) = project.media.iter_mut().find(|m| m.id == id)
        {
            m.bin = bin;
            out.modified = true;
        }
        if let Some(id) = remove {
            if project.media_in_use(id) {
                out.error = Some("Ese clip se usa en la línea de tiempo: quítalo de ahí primero.".into());
            } else {
                project.media.retain(|m| m.id != id);
                self.infos.remove(&id);
                self.thumbs.remove(&id);
                self.peaks.remove(&id);
                if self.selected == Some(id) {
                    self.selected = None;
                }
                out.removed = Some(id);
                out.modified = true;
            }
        }
    }

    /// Ficha del clip seleccionado, para la página Medios.
    pub fn metadata_ui(&self, ui: &mut egui::Ui, project: &Project) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Metadatos").color(theme::TEXT_BRIGHT));
        });
        ui.add_space(2.0);
        ui.separator();
        let Some(m) = self.selected.and_then(|id| project.media.iter().find(|m| m.id == id)) else {
            ui.add_space(12.0);
            ui.vertical_centered(|ui| ui.label(egui::RichText::new("Ningún clip seleccionado").color(theme::TEXT_DIM)));
            return;
        };
        let fps = project.settings.fps.as_f64();
        let mut rows: Vec<(&str, String)> =
            vec![("Nombre", m.name.clone()), ("Duración", timecode(m.duration as f64 / fps, fps))];
        if let Some(i) = self.infos.get(&m.id) {
            rows.push(("Resolución", format!("{} × {}", i.width, i.height)));
            rows.push(("Fotogramas/s", format!("{:.3}", i.fps)));
            rows.push(("Códec de vídeo", i.video_codec.clone()));
            rows.push(("Códec de audio", i.audio_codec.clone().unwrap_or_else(|| "sin audio".into())));
            if let Some(r) = i.audio_rate {
                rows.push(("Audio", format!("{r} Hz")));
            }
        }
        let bin = m.bin.and_then(|b| project.bins.iter().find(|x| x.id == b)).map_or("Principal", |b| b.name.as_str());
        rows.push(("Carpeta", bin.to_owned()));
        rows.push(("Archivo", m.path.display().to_string()));
        egui::ScrollArea::vertical().id_salt("ficha").show(ui, |ui| {
            ui.add_space(6.0);
            egui::Grid::new("metadatos").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                for (k, v) in rows {
                    ui.label(egui::RichText::new(format!("  {k}")).color(theme::TEXT_DIM));
                    ui.add(egui::Label::new(v).wrap());
                    ui.end_row();
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconoce_los_ritmos_ntsc_y_los_enteros() {
        assert_eq!(fps_rational(23.976), Rational::new(24000, 1001));
        assert_eq!(fps_rational(29.97003), Rational::new(30000, 1001));
        assert_eq!(fps_rational(59.94), Rational::new(60000, 1001));
        assert_eq!(fps_rational(25.0), Rational::new(25, 1));
        assert_eq!(fps_rational(0.0), Rational::new(30, 1));
    }

    #[test]
    fn segundos_y_fotogramas_van_y_vuelven() {
        let fps = Rational::new(25, 1);
        assert_eq!(secs_to_frames(1.24, fps), 31);
        assert!((frames_to_secs(31, fps) - 1.24).abs() < 1e-9);
    }
}
