#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod media_pool;
mod pages;
mod program;
mod timeline_view;
mod viewer;

use std::path::{Path, PathBuf};

use cinder_core::{Clip, ClipId, Edit, EditError, History, Project, TrackKind, ops};
use eframe::egui::{self, Color32, Key, Modifiers, RichText};

use media_pool::{MediaPool, secs_to_frames};
use pages::Page;
use program::Program;
use timeline_view::TimelineView;
use viewer::Viewer;

const ACCENT: Color32 = Color32::from_rgb(0x7B, 0x61, 0xFF);

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Cinder Studio")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([960.0, 600.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native("Cinder Studio", options, Box::new(|cc| Ok(Box::new(CinderApp::new(cc)))))
}

/// Qué visor recibe Espacio, J/K/L y las flechas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Monitor {
    Source,
    Program,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceEdit {
    Insert,
    Overwrite,
    Append,
}

struct CinderApp {
    project: Project,
    /// Carpeta `.cinder` donde se guardó o de donde se abrió.
    project_dir: Option<PathBuf>,
    history: History,
    /// Hay cambios sin guardar.
    modified: bool,
    page: Page,
    source: Viewer,
    program: Program,
    pool: MediaPool,
    timeline: TimelineView,
    timeline_width: f32,
    active: Monitor,
    status: Option<String>,
}

/// La tecla física (para que «,» con Mayús siga siendo «,» en cualquier distribución) con
/// exactamente esos modificadores.
fn pressed(ctx: &egui::Context, key: Key, mods: Modifiers) -> bool {
    ctx.input(|i| {
        i.events.iter().any(|e| match e {
            egui::Event::Key { key: k, physical_key, pressed: true, modifiers, .. } => {
                physical_key.unwrap_or(*k) == key && modifiers.matches_exact(mods)
            }
            _ => false,
        })
    })
}

impl CinderApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.selection.bg_fill = ACCENT;
        visuals.hyperlink_color = ACCENT;
        cc.egui_ctx.set_visuals(visuals);
        let mut app = Self {
            project: Project::new("Proyecto sin título"),
            project_dir: None,
            history: History::default(),
            modified: false,
            page: Page::Edicion,
            source: Viewer::new(),
            program: Program::new(),
            pool: MediaPool::default(),
            timeline: TimelineView::new(),
            timeline_width: 800.0,
            active: Monitor::Source,
            status: None,
        };
        // `cinder-studio clip.mp4` (y «Abrir con» del sistema) importa el clip y lo abre.
        let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
        if !args.is_empty() {
            app.import(&args);
        }
        app
    }

    fn import(&mut self, paths: &[PathBuf]) {
        if let Some(id) = self.pool.import(&mut self.project, paths) {
            self.modified = true;
            self.open_in_source(id);
        }
    }

    fn open_in_source(&mut self, id: cinder_core::MediaId) {
        if let Some(m) = self.project.media.iter().find(|m| m.id == id) {
            let path = m.path.clone();
            if self.source.path() != Some(path.as_path()) {
                self.source.open(&path);
            }
            self.active = Monitor::Source;
        }
    }

    fn apply(&mut self, edit: Edit) {
        match self.history.apply(&mut self.project.timeline, edit) {
            Ok(()) => self.edited(),
            Err(e) => self.status = Some(e.to_string()),
        }
    }

    fn edited(&mut self) {
        self.modified = true;
        self.status = None;
        self.program.invalidate();
        // El clip seleccionado puede haber desaparecido (deshacer, borrar).
        if let Some((t, id)) = self.timeline.selected
            && self.project.timeline.tracks.get(t).and_then(|tr| tr.clip(id)).is_none()
        {
            self.timeline.selected = None;
        }
    }

    fn first_track(&self, kind: TrackKind) -> Option<usize> {
        self.project.timeline.tracks.iter().position(|t| t.kind == kind)
    }

    /// F9, F10 y Mayús+F12: el tramo marcado en el visor de origen a V1 (y su audio a A1).
    fn edit_from_source(&mut self, mode: SourceEdit) {
        let result = self.build_source_edit(mode);
        match result {
            Ok((edit, end)) => {
                if self
                    .history
                    .apply(&mut self.project.timeline, edit)
                    .map_err(|e| self.status = Some(e.to_string()))
                    .is_ok()
                {
                    self.edited();
                    self.program.seek(end);
                    self.active = Monitor::Program;
                }
            }
            Err(msg) => self.status = Some(msg),
        }
    }

    fn build_source_edit(&mut self, mode: SourceEdit) -> Result<(Edit, cinder_core::Frames), String> {
        let path = self.source.path().ok_or("Abre un clip en el visor de origen primero.")?.to_path_buf();
        let media = self.pool.id_of(&self.project, &path).ok_or("El clip del visor no está en el proyecto.")?;
        let (a, b) = self.source.marked_range().ok_or("El tramo marcado está vacío.")?;
        let fps = self.project.settings.fps;
        let media_len = self.project.media.iter().find(|m| m.id == media).map_or(0, |m| m.duration);
        let source_in = secs_to_frames(a, fps).clamp(0, media_len);
        let duration = (secs_to_frames(b, fps) - source_in).min(media_len - source_in);
        if duration <= 0 {
            return Err("El tramo marcado está vacío.".into());
        }
        let at = match mode {
            SourceEdit::Append => self.project.timeline.duration(),
            SourceEdit::Insert | SourceEdit::Overwrite => self.program.playhead,
        };
        let has_audio = self.pool.info(media).is_some_and(|i| i.audio_codec.is_some());
        let mut targets = vec![self.first_track(TrackKind::Video).ok_or("No hay pista de vídeo.")?];
        if has_audio && let Some(a) = self.first_track(TrackKind::Audio) {
            targets.push(a);
        }
        let mut edits = Vec::new();
        for track in targets {
            let clip = Clip { id: ClipId(self.project.alloc_id()), media, start: at, source_in, duration };
            let split_id = ClipId(self.project.alloc_id());
            let edit = match mode {
                SourceEdit::Insert | SourceEdit::Append => ops::insert(&self.project, track, clip, split_id),
                SourceEdit::Overwrite => ops::overwrite(&self.project, track, clip, split_id),
            };
            edits.push(edit.map_err(|e| e.to_string())?);
        }
        Ok((Edit::Batch(edits), at + duration))
    }

    /// Ctrl+B: corta en el cabezal todas las pistas que tengan un clip ahí.
    fn blade(&mut self) {
        let at = self.program.playhead;
        let mut edits = Vec::new();
        for track in 0..self.project.timeline.tracks.len() {
            let id = ClipId(self.project.alloc_id());
            if let Ok(e) = ops::split(&self.project, track, at, id) {
                edits.push(e);
            }
        }
        if edits.is_empty() {
            self.status = Some(EditError::NothingThere.to_string());
        } else {
            self.apply(Edit::Batch(edits));
        }
    }

    /// Operación sobre el clip seleccionado (y su pareja de audio o vídeo).
    fn on_selected(&mut self, f: impl Fn(&Project, usize, ClipId) -> Result<Edit, EditError>) {
        let Some((track, id)) = self.timeline.selected else {
            self.status = Some("Selecciona un clip en la línea de tiempo.".into());
            return;
        };
        match ops::on_linked(&self.project, track, id, f) {
            Ok(e) => self.apply(e),
            Err(e) => self.status = Some(e.to_string()),
        }
    }

    /// Salta al punto de corte anterior o siguiente (↑ / ↓, como en Resolve).
    fn jump_edit(&mut self, forward: bool) {
        let at = self.program.playhead;
        let points =
            self.project.timeline.tracks.iter().flat_map(|t| t.clips().iter().flat_map(|c| [c.start, c.end()]));
        let target = if forward { points.filter(|&p| p > at).min() } else { points.filter(|&p| p < at).max() };
        if let Some(p) = target {
            self.program.seek(p);
        }
    }

    fn undo(&mut self, redo: bool) {
        let r = if redo {
            self.history.redo(&mut self.project.timeline)
        } else {
            self.history.undo(&mut self.project.timeline)
        };
        match r {
            Ok(true) => self.edited(),
            Ok(false) => {}
            Err(e) => self.status = Some(e.to_string()),
        }
    }

    fn save(&mut self, ask: bool) {
        let dir = match (&self.project_dir, ask) {
            (Some(d), false) => Some(d.clone()),
            _ => rfd::FileDialog::new()
                .set_title("Guardar proyecto")
                .set_file_name(format!("{}.cinder", self.project.name))
                .save_file()
                .map(|p| if p.extension().is_some_and(|e| e == "cinder") { p } else { p.with_extension("cinder") }),
        };
        let Some(dir) = dir else { return };
        if let Some(stem) = dir.file_stem() {
            self.project.name = stem.to_string_lossy().into_owned();
        }
        match self.project.save(&dir) {
            Ok(()) => {
                self.status = Some(format!("Guardado en {}", dir.display()));
                self.project_dir = Some(dir);
                self.modified = false;
            }
            Err(e) => self.status = Some(e.to_string()),
        }
    }

    fn open_project(&mut self) {
        let Some(dir) = rfd::FileDialog::new().set_title("Abrir proyecto (carpeta .cinder)").pick_folder() else {
            return;
        };
        self.load_project(&dir);
    }

    fn load_project(&mut self, dir: &Path) {
        match Project::load(dir) {
            Ok(p) => {
                self.project = p;
                self.project_dir = Some(dir.to_path_buf());
                self.history = History::default();
                self.modified = false;
                self.pool.reload(&self.project);
                self.program.forget_media();
                self.program.seek(0);
                self.source = Viewer::new();
                self.timeline.selected = None;
                self.status = Some(format!("Abierto {}", dir.display()));
            }
            Err(e) => self.status = Some(e.to_string()),
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        // Con Mayús, la tecla 6 llega como «&» (y así en cada distribución); se mira la tecla física.
        let page = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, physical_key, pressed: true, repeat: false, modifiers, .. }
                    if modifiers.shift && !modifiers.command =>
                {
                    let k = physical_key.unwrap_or(*key);
                    Page::ALL.into_iter().find(|p| p.key() == k)
                }
                _ => None,
            })
        });
        if let Some(page) = page {
            self.page = page;
        }

        // Archivos soltados en la ventana: al proyecto y al visor de origen.
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect()
        });
        if !dropped.is_empty() {
            self.import(&dropped);
        }

        let cmd = Modifiers::COMMAND;
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        if pressed(ctx, Key::I, cmd) {
            let paths = Viewer::pick_paths();
            self.import(&paths);
        }
        if std::mem::take(&mut self.source.wants_import) {
            let paths = Viewer::pick_paths();
            self.import(&paths);
        }
        if pressed(ctx, Key::S, cmd) {
            self.save(false);
        }
        if pressed(ctx, Key::S, cmd_shift) {
            self.save(true);
        }
        if pressed(ctx, Key::O, cmd) {
            self.open_project();
        }
        if ctx.egui_wants_keyboard_input() || !self.has_viewer() {
            return;
        }
        if pressed(ctx, Key::Z, cmd) {
            self.undo(false);
        }
        if pressed(ctx, Key::Z, cmd_shift) || pressed(ctx, Key::Y, cmd) {
            self.undo(true);
        }
        if self.page != Page::Edicion {
            return;
        }
        let none = Modifiers::NONE;
        if pressed(ctx, Key::F9, none) {
            self.edit_from_source(SourceEdit::Insert);
        }
        if pressed(ctx, Key::F10, none) {
            self.edit_from_source(SourceEdit::Overwrite);
        }
        if pressed(ctx, Key::F12, Modifiers::SHIFT) {
            self.edit_from_source(SourceEdit::Append);
        }
        if pressed(ctx, Key::B, cmd) {
            self.blade();
        }
        if pressed(ctx, Key::Backspace, none) {
            self.on_selected(|_, track, id| Ok(Edit::RemoveClip { track, id }));
        }
        if pressed(ctx, Key::Delete, none) || pressed(ctx, Key::Backspace, Modifiers::SHIFT) {
            self.on_selected(ops::ripple_delete);
        }
        // , y . empujan el clip un fotograma; con Alt, slip; con Mayús, slide; con Ctrl, roll del corte de su derecha.
        for (key, d) in [(Key::Comma, -1), (Key::Period, 1)] {
            if pressed(ctx, key, none) {
                self.on_selected(|p, t, id| {
                    let c = p.timeline.track(t)?.clip(id).ok_or(EditError::NoSuchClip(id))?;
                    Ok(Edit::MoveClip { track: t, id, to: c.start + d })
                });
            }
            if pressed(ctx, key, Modifiers::ALT) {
                self.on_selected(|p, t, id| ops::slip(p, t, id, d));
            }
            if pressed(ctx, key, Modifiers::SHIFT) {
                self.on_selected(|p, t, id| ops::slide(p, t, id, d));
            }
            if pressed(ctx, key, cmd) {
                self.on_selected(|p, t, id| ops::roll(p, t, id, d));
            }
        }
        if pressed(ctx, Key::ArrowUp, none) {
            self.jump_edit(false);
        }
        if pressed(ctx, Key::ArrowDown, none) {
            self.jump_edit(true);
        }
        if pressed(ctx, Key::Equals, cmd) || pressed(ctx, Key::Plus, cmd) {
            self.timeline.zoom_by(1.5);
        }
        if pressed(ctx, Key::Minus, cmd) {
            self.timeline.zoom_by(1.0 / 1.5);
        }
        if pressed(ctx, Key::Z, Modifiers::SHIFT) {
            self.timeline.fit(self.project.timeline.duration(), self.timeline_width);
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let s = &self.project.settings;
        let info = format!("{}×{} · {:.3} fps · {} Hz", s.width, s.height, s.fps.as_f64(), s.sample_rate);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Cinder Studio").strong().color(ACCENT));
            ui.separator();
            let star = if self.modified { " •" } else { "" };
            ui.label(format!("{}{star}", self.project.name));
            if ui.small_button("Abrir…").on_hover_text("Ctrl+O").clicked() {
                self.open_project();
            }
            if ui.small_button("Guardar").on_hover_text("Ctrl+S · Ctrl+Mayús+S guarda como").clicked() {
                self.save(false);
            }
            if let Some(msg) = &self.status {
                ui.separator();
                ui.colored_label(Color32::from_rgb(0xF5, 0xA6, 0x23), msg);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(info);
            });
        });
    }

    fn page_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            for page in Page::ALL {
                let text = RichText::new(page.label()).size(15.0);
                if ui.selectable_label(self.page == page, text).on_hover_text(page.shortcut_label()).clicked() {
                    self.page = page;
                }
            }
        });
    }

    fn has_viewer(&self) -> bool {
        matches!(self.page, Page::Medios | Page::Edicion)
    }

    fn medios_page(&mut self, ui: &mut egui::Ui) {
        let mut open = None;
        egui::Panel::left("panel-medios").resizable(true).default_size(340.0).show(ui, |ui| {
            open = self.pool.ui(ui, &self.project);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            if self.source.ui(ui) {
                self.active = Monitor::Source;
            }
        });
        if let Some(id) = open {
            self.open_in_source(id);
        }
    }

    fn edicion_page(&mut self, ui: &mut egui::Ui) {
        let mut tl_out = timeline_view::Response::default();
        egui::Panel::bottom("linea-de-tiempo").resizable(true).default_size(260.0).min_size(140.0).show(ui, |ui| {
            tl_out = self.timeline.ui(ui, &self.project, self.program.playhead, &mut self.timeline_width);
        });
        let mut open = None;
        egui::Panel::left("panel-medios-edicion").resizable(true).default_size(240.0).show(ui, |ui| {
            open = self.pool.ui(ui, &self.project);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            let half = (ui.available_width() - 12.0) / 2.0;
            let h = ui.available_height();
            ui.horizontal(|ui| {
                let active_frame = |active: bool| {
                    egui::Frame::new()
                        .stroke(egui::Stroke::new(1.0, if active { ACCENT } else { Color32::from_gray(50) }))
                        .inner_margin(4.0)
                };
                active_frame(self.active == Monitor::Source).show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(half - 8.0);
                        ui.set_height(h - 8.0);
                        ui.weak("Origen");
                        if self.source.ui(ui) {
                            self.active = Monitor::Source;
                        }
                    });
                });
                active_frame(self.active == Monitor::Program).show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(half - 8.0);
                        ui.set_height(h - 8.0);
                        ui.weak("Programa");
                        if self.program.ui(ui, &self.project) {
                            self.active = Monitor::Program;
                        }
                    });
                });
            });
        });
        if let Some(id) = open {
            self.open_in_source(id);
        }
        if tl_out.focused {
            self.active = Monitor::Program;
            self.source.pause();
        }
        if let Some(f) = tl_out.seek {
            self.program.seek(f);
        }
        if let Some(e) = tl_out.error {
            self.status = Some(e.to_string());
        }
        if let Some(edit) = tl_out.edit {
            self.apply(edit);
        }
    }

    fn page_body(&mut self, ui: &mut egui::Ui) {
        match self.page {
            Page::Medios => self.medios_page(ui),
            Page::Edicion => self.edicion_page(ui),
            other => {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(ui.available_height() * 0.3);
                        ui.heading(RichText::new(other.label()).size(32.0));
                        ui.add_space(8.0);
                        ui.label(other.summary());
                        ui.add_space(4.0);
                        ui.weak(format!("Esta página llega en la fase {} del roadmap.", other.phase()));
                    });
                });
            }
        }
    }
}

impl eframe::App for CinderApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_shortcuts(ctx);
        if self.has_viewer() {
            match (self.page, self.active) {
                (Page::Edicion, Monitor::Program) => self.program.handle_input(ctx, &self.project),
                _ => self.source.handle_input(ctx),
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("barra-superior").show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("barra-paginas").exact_size(44.0).show(ui, |ui| self.page_bar(ui));
        self.page_body(ui);
    }
}
