//! Página Entrega: elegir un preset, añadir la línea de tiempo a la cola de render y
//! renderizar los trabajos uno detrás de otro en un hilo aparte.
//!
//! Cada trabajo guarda una foto de la línea de tiempo en el momento de añadirlo, así que se
//! puede seguir editando mientras la cola trabaja.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::JoinHandle;
use std::time::Duration;

use cinder_core::deliver::{Span, audio_spans, video_spans};
use cinder_core::{Frames, Project};
use cinder_media::{ExportPreset, ExportReport, MediaError, RenderPlan, RenderSpan};
use eframe::egui::{self, Align, Layout, RichText, Vec2};

use crate::theme;

#[derive(Debug, Clone, PartialEq)]
enum JobState {
    Waiting,
    Rendering,
    Done { seconds: f64, encoder: String },
    Failed(String),
    Cancelled,
}

struct Job {
    id: u64,
    output: PathBuf,
    preset: String,
    summary: String,
    plan: RenderPlan,
    settings: cinder_media::ExportSettings,
    state: JobState,
}

struct Worker {
    job: u64,
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicI64>,
    total: Arc<AtomicI64>,
    result: Receiver<Result<ExportReport, MediaError>>,
    handle: Option<JoinHandle<()>>,
}

pub struct Deliver {
    presets: Vec<ExportPreset>,
    selected: usize,
    file_name: String,
    folder: Option<PathBuf>,
    jobs: Vec<Job>,
    next_id: u64,
    worker: Option<Worker>,
    /// La cola sigue con el siguiente trabajo al acabar uno.
    running: bool,
    message: Option<String>,
}

impl Default for Deliver {
    fn default() -> Self {
        Self {
            presets: ExportPreset::all(),
            selected: 0,
            file_name: String::new(),
            folder: None,
            jobs: Vec::new(),
            next_id: 1,
            worker: None,
            running: false,
            message: None,
        }
    }
}

/// Pasa la línea de tiempo del proyecto a un plan de render con la resolución y los
/// fotogramas por segundo del preset.
pub fn build_plan(project: &Project, project_dir: Option<&Path>, preset: &ExportPreset) -> Result<RenderPlan, String> {
    let s = &project.settings;
    let ((width, height), fps) = preset.resolve((s.width, s.height), (s.fps.num, s.fps.den));
    // Fotograma del proyecto → fotograma de salida (iguales salvo que el preset baje los fps).
    let scale = |f: Frames| -> i64 {
        let num = i128::from(f) * i128::from(fps.0) * i128::from(s.fps.den);
        let den = i128::from(fps.1) * i128::from(s.fps.num.max(1));
        ((num + den / 2) / den.max(1)) as i64
    };
    let resolve = |span: &Span| -> Option<RenderSpan> {
        let item = project.media.iter().find(|m| m.id == span.media)?;
        let path = match project_dir {
            Some(dir) if item.path.is_relative() => dir.join(&item.path),
            _ => item.path.clone(),
        };
        let start = scale(span.start);
        let frames = scale(span.end()) - start;
        (frames > 0).then(|| RenderSpan { path, start, source_secs: span.source_in as f64 / s.fps.as_f64(), frames })
    };
    let frames = scale(project.timeline.duration());
    if frames <= 0 {
        return Err("La línea de tiempo está vacía: monta algo en la página Edición primero.".into());
    }
    Ok(RenderPlan {
        width,
        height,
        fps,
        sample_rate: s.sample_rate,
        frames,
        video: video_spans(&project.timeline).iter().filter_map(resolve).collect(),
        audio: audio_spans(&project.timeline).iter().filter_map(resolve).collect(),
    })
}

fn fmt_duration(secs: f64) -> String {
    let t = secs.max(0.0).round() as u64;
    if t >= 3600 {
        format!("{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60)
    } else {
        format!("{}:{:02}", t / 60, t % 60)
    }
}

impl Deliver {
    /// Recoge el resultado del trabajo en marcha y arranca el siguiente. Llamar en cada repintado.
    pub fn update(&mut self, ctx: &egui::Context) {
        if let Some(w) = self.worker.as_mut() {
            let outcome = match w.result.try_recv() {
                Ok(r) => Some(r),
                Err(TryRecvError::Disconnected) => Some(Err(MediaError::Thread("el render se cortó".into()))),
                Err(TryRecvError::Empty) => None,
            };
            if let Some(outcome) = outcome {
                let id = w.job;
                if let Some(h) = w.handle.take() {
                    h.join().ok();
                }
                self.worker = None;
                if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
                    job.state = match outcome {
                        Ok(r) => JobState::Done { seconds: r.seconds, encoder: r.encoder },
                        Err(MediaError::Cancelled) => JobState::Cancelled,
                        Err(e) => JobState::Failed(e.to_string()),
                    };
                }
            }
        }
        if self.worker.is_none() && self.running {
            match self.jobs.iter().position(|j| j.state == JobState::Waiting) {
                Some(i) => self.start(i),
                None => self.running = false,
            }
        }
        // Mientras haya un render en marcha, la barra de progreso se repinta sola.
        if self.worker.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn start(&mut self, index: usize) {
        let Some(job) = self.jobs.get_mut(index) else { return };
        job.state = JobState::Rendering;
        let (cancel, done, total) =
            (Arc::new(AtomicBool::new(false)), Arc::new(AtomicI64::new(0)), Arc::new(AtomicI64::new(job.plan.frames)));
        let (tx, rx) = mpsc::channel();
        let (plan, settings, output) = (job.plan.clone(), job.settings.clone(), job.output.clone());
        let (c, d, t) = (cancel.clone(), done.clone(), total.clone());
        let handle = std::thread::Builder::new().name("cinder-render".into()).spawn(move || {
            let r = cinder_media::render(&plan, &settings, &output, &c, |n, all| {
                d.store(n, Ordering::Relaxed);
                t.store(all, Ordering::Relaxed);
            });
            tx.send(r).ok();
        });
        match handle {
            Ok(h) => {
                self.worker = Some(Worker { job: job.id, cancel, done, total, result: rx, handle: Some(h) });
            }
            Err(e) => job.state = JobState::Failed(format!("no se pudo arrancar el render: {e}")),
        }
    }

    fn add_job(&mut self, project: &Project, project_dir: Option<&Path>) {
        self.message = None;
        let Some(preset) = self.presets.get(self.selected) else { return };
        let plan = match build_plan(project, project_dir, preset) {
            Ok(p) => p,
            Err(e) => {
                self.message = Some(e);
                return;
            }
        };
        let folder = self
            .folder
            .clone()
            .or_else(|| project_dir.and_then(Path::parent).map(Path::to_path_buf))
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default();
        let name =
            if self.file_name.trim().is_empty() { project.name.clone() } else { self.file_name.trim().to_string() };
        let mut output = folder.join(format!("{name}.mp4"));
        // No pisar otro trabajo de la cola ni un archivo que ya existe.
        let mut n = 2;
        while output.exists() || self.jobs.iter().any(|j| j.output == output && j.state == JobState::Waiting) {
            output = folder.join(format!("{name} ({n}).mp4"));
            n += 1;
        }
        let secs = plan.frames as f64 * f64::from(plan.fps.1) / f64::from(plan.fps.0.max(1));
        let summary = format!(
            "{}×{} · {:.2} fps · {} · {}",
            plan.width,
            plan.height,
            f64::from(plan.fps.0) / f64::from(plan.fps.1.max(1)),
            preset.settings.codec.label(),
            fmt_duration(secs)
        );
        self.jobs.push(Job {
            id: self.next_id,
            output,
            preset: preset.name.to_string(),
            summary,
            plan,
            settings: preset.settings.clone(),
            state: JobState::Waiting,
        });
        self.next_id += 1;
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, project: &Project, project_dir: Option<&Path>) {
        egui::Panel::left("entrega-ajustes")
            .resizable(true)
            .default_size(360.0)
            .min_size(300.0)
            .frame(theme::flush(theme::PANEL).inner_margin(14))
            .show(ui, |ui| self.settings_ui(ui, project, project_dir));
        egui::CentralPanel::default().frame(theme::flush(theme::BG).inner_margin(14)).show(ui, |ui| self.queue_ui(ui));
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui, project: &Project, project_dir: Option<&Path>) {
        ui.label(RichText::new("Ajustes de render").size(16.0).color(theme::TEXT_BRIGHT));
        ui.add_space(10.0);
        for (i, p) in self.presets.iter().enumerate() {
            let selected = i == self.selected;
            let frame = egui::Frame::new()
                .fill(if selected { theme::RAISED_HOVER } else { theme::RAISED })
                .stroke(egui::Stroke::new(1.0, if selected { theme::BRAND } else { theme::LINE }))
                .corner_radius(6)
                .inner_margin(10);
            let resp = frame
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(p.name).strong().color(theme::TEXT_BRIGHT));
                    ui.label(RichText::new(p.description).small().color(theme::TEXT_DIM));
                })
                .response
                .interact(egui::Sense::click());
            if resp.clicked() {
                self.selected = i;
            }
            ui.add_space(6.0);
        }
        ui.add_space(8.0);
        ui.label("Nombre del archivo");
        ui.add(
            egui::TextEdit::singleline(&mut self.file_name)
                .hint_text(project.name.as_str())
                .desired_width(f32::INFINITY),
        );
        ui.add_space(6.0);
        ui.label("Carpeta");
        ui.horizontal(|ui| {
            let shown = self
                .folder
                .as_ref()
                .map(|f| f.display().to_string())
                .unwrap_or_else(|| "Junto al proyecto".to_string());
            if ui.button("Elegir…").clicked()
                && let Some(dir) = rfd::FileDialog::new().set_title("Carpeta de salida").pick_folder()
            {
                self.folder = Some(dir);
            }
            ui.label(RichText::new(shown).color(theme::TEXT_DIM));
        });
        ui.add_space(14.0);
        if ui
            .add_sized(Vec2::new(ui.available_width(), 32.0), egui::Button::new("Añadir a la cola de render"))
            .clicked()
        {
            self.add_job(project, project_dir);
        }
        if let Some(m) = &self.message {
            ui.add_space(6.0);
            ui.label(RichText::new(m).color(theme::WARN));
        }
    }

    fn queue_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Cola de render").size(16.0).color(theme::TEXT_BRIGHT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let waiting = self.jobs.iter().any(|j| j.state == JobState::Waiting);
                if self.running || self.worker.is_some() {
                    if ui.button("Parar").clicked() {
                        self.running = false;
                        if let Some(w) = &self.worker {
                            w.cancel.store(true, Ordering::Relaxed);
                        }
                    }
                } else if ui.add_enabled(waiting, egui::Button::new("Renderizar todo")).clicked() {
                    self.running = true;
                }
                let finished = self.jobs.iter().any(|j| !matches!(j.state, JobState::Waiting | JobState::Rendering));
                if ui.add_enabled(finished, egui::Button::new("Limpiar acabados")).clicked() {
                    self.jobs.retain(|j| matches!(j.state, JobState::Waiting | JobState::Rendering));
                }
            });
        });
        ui.add_space(10.0);
        if self.jobs.is_empty() {
            ui.label(
                RichText::new("No hay nada en la cola. Elige un preset y pulsa «Añadir a la cola de render».")
                    .color(theme::TEXT_DIM),
            );
            return;
        }
        let progress = self.worker.as_ref().map(|w| {
            let total = w.total.load(Ordering::Relaxed).max(1);
            (w.job, w.done.load(Ordering::Relaxed) as f32 / total as f32)
        });
        let mut remove = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, job) in self.jobs.iter().enumerate() {
                egui::Frame::new().fill(theme::PANEL).corner_radius(6).inner_margin(10).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let name = job.output.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        ui.label(RichText::new(name).strong().color(theme::TEXT_BRIGHT));
                        ui.label(RichText::new(&job.preset).color(theme::TEXT_DIM));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| match &job.state {
                            JobState::Rendering => {
                                if ui.button("Cancelar").clicked()
                                    && let Some(w) = &self.worker
                                {
                                    w.cancel.store(true, Ordering::Relaxed);
                                }
                            }
                            _ => {
                                if ui.button("Quitar").clicked() {
                                    remove = Some(i);
                                }
                            }
                        });
                    });
                    ui.label(RichText::new(&job.summary).small().color(theme::TEXT_DIM));
                    ui.label(RichText::new(job.output.display().to_string()).small().color(theme::TEXT_DIM));
                    ui.add_space(4.0);
                    match &job.state {
                        JobState::Waiting => {
                            ui.label("En espera");
                        }
                        JobState::Rendering => {
                            let p = progress.filter(|(id, _)| *id == job.id).map_or(0.0, |(_, p)| p);
                            ui.add(egui::ProgressBar::new(p).show_percentage().fill(theme::BRAND));
                        }
                        JobState::Done { seconds, encoder } => {
                            ui.label(
                                RichText::new(format!("Hecho en {} · {encoder}", fmt_duration(*seconds)))
                                    .color(theme::CLIP_AUDIO),
                            );
                        }
                        JobState::Failed(e) => {
                            ui.label(RichText::new(e).color(theme::ERROR));
                        }
                        JobState::Cancelled => {
                            ui.label(RichText::new("Cancelado").color(theme::WARN));
                        }
                    }
                });
                ui.add_space(8.0);
            }
        });
        if let Some(i) = remove {
            self.jobs.remove(i);
        }
    }
}

impl Drop for Deliver {
    fn drop(&mut self) {
        // Al cerrar la app con un render en marcha: pararlo y no dejar un archivo a medias.
        if let Some(w) = self.worker.as_mut() {
            w.cancel.store(true, Ordering::Relaxed);
            if let Some(h) = w.handle.take() {
                h.join().ok();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cinder_core::{Clip, ClipId, MediaId, MediaItem, Rational};

    fn project() -> Project {
        let mut p = Project::new("prueba");
        p.settings.fps = Rational::new(60, 1);
        p.media.push(MediaItem {
            id: MediaId(1),
            name: "a.mp4".into(),
            path: "a.mp4".into(),
            duration: 600,
            bin: None,
        });
        let clip = Clip { id: ClipId(1), media: MediaId(1), start: 30, source_in: 120, duration: 90, link: None };
        p.timeline.tracks[0].insert(clip.clone()).unwrap();
        p.timeline.tracks[1].insert(Clip { id: ClipId(2), ..clip }).unwrap();
        p
    }

    #[test]
    fn el_plan_pasa_a_los_fps_y_la_resolucion_del_preset() {
        let p = project();
        let presets = ExportPreset::all();
        let youtube = presets.iter().find(|x| x.name == "YouTube 1080p").unwrap();
        let plan = build_plan(&p, Some(Path::new("/proyectos")), youtube).unwrap();
        assert_eq!((plan.width, plan.height, plan.fps, plan.frames), (1920, 1080, (60, 1), 120));
        let v = &plan.video[0];
        assert_eq!((v.start, v.frames), (30, 90));
        assert!((v.source_secs - 2.0).abs() < 1e-9);
        assert_eq!(v.path, Path::new("/proyectos/a.mp4"));
        assert_eq!(plan.audio.len(), 1);
    }

    #[test]
    fn tiktok_a_30_fps_divide_los_fotogramas() {
        let mut p = project();
        p.settings.fps = Rational::new(120, 1);
        let presets = ExportPreset::all();
        let tiktok = presets.iter().find(|x| x.name.starts_with("TikTok")).unwrap();
        let plan = build_plan(&p, None, tiktok).unwrap();
        assert_eq!(plan.fps, (120, 2));
        assert_eq!((plan.video[0].start, plan.video[0].frames, plan.frames), (15, 45, 60));
        // El segundo del origen no cambia: 120 fotogramas a 120 fps = 1 s.
        assert!((plan.video[0].source_secs - 1.0).abs() < 1e-9);
    }

    #[test]
    fn una_linea_de_tiempo_vacia_no_se_puede_entregar() {
        let p = Project::new("vacío");
        assert!(build_plan(&p, None, &ExportPreset::all()[0]).is_err());
    }
}
