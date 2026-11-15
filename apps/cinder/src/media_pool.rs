//! Panel de medios: lo importado al proyecto, con su ficha. Un clic lo abre en el visor de origen.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use cinder_core::{Frames, MediaId, MediaItem, Project, Rational};
use cinder_media::MediaInfo;
use eframe::egui::{self, Color32, RichText};

use crate::viewer::timecode;

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

#[derive(Default)]
pub struct MediaPool {
    infos: HashMap<MediaId, MediaInfo>,
    errors: Vec<String>,
    pub selected: Option<MediaId>,
}

impl MediaPool {
    pub fn info(&self, id: MediaId) -> Option<&MediaInfo> {
        self.infos.get(&id)
    }

    pub fn id_of(&self, project: &Project, path: &Path) -> Option<MediaId> {
        project.media.iter().find(|m| m.path == path).map(|m| m.id)
    }

    /// Importa los archivos y devuelve el último que se pudo abrir (para enseñarlo en el visor).
    pub fn import(&mut self, project: &mut Project, paths: &[PathBuf]) -> Option<MediaId> {
        let mut last = None;
        for path in paths {
            if let Some(id) = self.id_of(project, path) {
                last = Some(id);
                continue;
            }
            match MediaInfo::probe(path) {
                Ok(info) => {
                    // El primer clip de un proyecto vacío fija su formato, como en Resolve.
                    if project.media.is_empty() && project.timeline.duration() == 0 && info.fps > 0.0 {
                        project.settings.fps = fps_rational(info.fps);
                        project.settings.width = info.width;
                        project.settings.height = info.height;
                    }
                    let id = MediaId(project.alloc_id());
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let duration = secs_to_frames(info.duration, project.settings.fps).max(1);
                    project.media.push(MediaItem { id, name, path: path.clone(), duration });
                    self.infos.insert(id, info);
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
        self.errors.clear();
        self.selected = None;
        for m in &project.media {
            match MediaInfo::probe(&m.path) {
                Ok(info) => {
                    self.infos.insert(m.id, info);
                }
                Err(e) => self.errors.push(format!("{}: {e}", m.name)),
            }
        }
    }

    /// Devuelve el medio en el que se hizo clic.
    pub fn ui(&mut self, ui: &mut egui::Ui, project: &Project) -> Option<MediaId> {
        let mut clicked = None;
        ui.horizontal(|ui| {
            ui.heading("Medios");
            ui.weak(format!("{}", project.media.len()));
        });
        ui.separator();
        if project.media.is_empty() {
            ui.add_space(12.0);
            ui.label("Todavía no hay nada importado.");
            ui.weak("Arrastra clips a la ventana o pulsa Ctrl+I.");
        }
        let fps = project.settings.fps.as_f64();
        egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
            for m in &project.media {
                let sel = self.selected == Some(m.id);
                let resp = ui
                    .add(
                        egui::Button::selectable(sel, RichText::new(&m.name).strong())
                            .min_size(egui::vec2(ui.available_width(), 0.0)),
                    )
                    .on_hover_text(m.path.display().to_string());
                let detail = match self.infos.get(&m.id) {
                    Some(i) => format!(
                        "{} · {}×{} · {:.3} fps · {}{}",
                        timecode(m.duration as f64 / fps, fps),
                        i.width,
                        i.height,
                        i.fps,
                        i.video_codec,
                        i.audio_codec.as_deref().map(|a| format!(" + {a}")).unwrap_or_default()
                    ),
                    None => "no se encuentra el archivo".into(),
                };
                ui.weak(detail);
                ui.add_space(4.0);
                if resp.clicked() {
                    self.selected = Some(m.id);
                    clicked = Some(m.id);
                }
            }
            for e in &self.errors {
                ui.colored_label(Color32::from_rgb(0xF0, 0x7F, 0x7F), e);
            }
        });
        clicked
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
