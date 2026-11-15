#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod icons;
mod media_pool;
mod monitor;
mod pages;
mod program;
mod theme;
mod timeline_view;
mod viewer;

use std::path::{Path, PathBuf};

use cinder_core::{Clip, ClipId, Edit, EditError, Frames, History, MediaId, Project, Track, TrackKind, ops};
use eframe::egui::{self, Align2, FontId, Key, Modifiers, Pos2, Rect, RichText, Sense, Vec2};

use icons::Icon;
use media_pool::{MediaPool, PoolOutput, secs_to_frames};
use pages::Page;
use program::Program;
use timeline_view::{Action, TimelineView, Tool};
use viewer::Viewer;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Cinder Studio")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1024.0, 640.0]),
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
enum Place {
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
    /// Panel de medios visible en la página Edición.
    show_pool: bool,
    /// Pistas de destino de insertar, sobrescribir y añadir.
    dest_video: usize,
    dest_audio: usize,
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
        theme::apply(&cc.egui_ctx);
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
            show_pool: true,
            dest_video: 0,
            dest_audio: 1,
            active: Monitor::Source,
            status: None,
        };
        // `cinder-studio clip.mp4` (y «Abrir con» del sistema) importa los clips;
        // `cinder-studio proyecto.cinder` abre el proyecto.
        let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
        match args.first() {
            Some(dir) if dir.join(cinder_core::project::PROJECT_FILE).is_file() => app.load_project(dir),
            Some(_) => app.import(&args),
            None => {}
        }
        app
    }

    fn import(&mut self, paths: &[PathBuf]) {
        if let Some(id) = self.pool.import(&mut self.project, paths) {
            self.modified = true;
            self.open_in_source(id);
        }
    }

    fn open_in_source(&mut self, id: MediaId) {
        if let Some(m) = self.project.media.iter().find(|m| m.id == id) {
            let path = m.path.clone();
            if self.source.path() != Some(path.as_path()) {
                self.source.pause(self.program.mixer());
                self.source.open(&path);
            }
            self.active = Monitor::Source;
        }
    }

    fn apply(&mut self, edit: Edit) -> bool {
        match self.history.apply(&mut self.project.timeline, edit) {
            Ok(()) => {
                self.edited();
                true
            }
            Err(e) => {
                self.status = Some(e.to_string());
                false
            }
        }
    }

    /// La línea de tiempo cambió: lo que dependía de ella se pone al día.
    fn edited(&mut self) {
        self.modified = true;
        self.status = None;
        self.program.invalidate();
        if let Some((t, id)) = self.timeline.selected
            && self.project.timeline.tracks.get(t).and_then(|tr| tr.clip(id)).is_none()
        {
            self.timeline.selected = None;
        }
        self.fix_dests();
    }

    /// Las pistas de destino tienen que existir y ser de su tipo.
    fn fix_dests(&mut self) {
        let tracks = &self.project.timeline.tracks;
        let first = |kind| tracks.iter().position(|t| t.kind == kind).unwrap_or(0);
        if tracks.get(self.dest_video).is_none_or(|t| t.kind != TrackKind::Video) {
            self.dest_video = first(TrackKind::Video);
        }
        if tracks.get(self.dest_audio).is_none_or(|t| t.kind != TrackKind::Audio) {
            self.dest_audio = first(TrackKind::Audio);
        }
    }

    /// F9, F10 y Mayús+F12: el tramo marcado en el visor de origen a las pistas de destino.
    fn place_from_source(&mut self, place: Place) {
        let fps = self.project.settings.fps;
        let found = (|| {
            let path = self.source.path().ok_or("Abre un clip en el visor de origen primero.")?;
            let media = self.pool.id_of(&self.project, path).ok_or("El clip del visor no está en el proyecto.")?;
            let (a, b) = self.source.marked_range().ok_or("El tramo marcado está vacío.")?;
            Ok::<_, &str>((media, secs_to_frames(a, fps), secs_to_frames(b, fps)))
        })();
        match found {
            Ok((media, a, b)) => {
                let at = match place {
                    Place::Append => self.project.timeline.duration(),
                    Place::Insert | Place::Overwrite => self.program.playhead,
                };
                self.place_media(media, a, b - a, at, place, true);
            }
            Err(msg) => self.status = Some(msg.into()),
        }
    }

    /// Coloca un tramo de un medio en las pistas de destino (vídeo y, si tiene, audio enlazado).
    #[allow(clippy::too_many_arguments)]
    fn place_media(
        &mut self,
        media: MediaId,
        source_in: Frames,
        duration: Frames,
        at: Frames,
        place: Place,
        move_playhead: bool,
    ) {
        let media_len = self.project.media.iter().find(|m| m.id == media).map_or(0, |m| m.duration);
        let source_in = source_in.clamp(0, media_len);
        let duration = duration.min(media_len - source_in);
        if duration <= 0 {
            self.status = Some("El tramo marcado está vacío.".into());
            return;
        }
        self.fix_dests();
        let has_audio = self.pool.info(media).is_some_and(|i| i.audio_codec.is_some());
        let mut targets = vec![self.dest_video];
        if has_audio {
            targets.push(self.dest_audio);
        }
        let link = has_audio.then(|| self.project.alloc_id());
        let mut edits = Vec::new();
        for track in targets {
            let clip = Clip { id: ClipId(self.project.alloc_id()), media, start: at, source_in, duration, link };
            let split_id = ClipId(self.project.alloc_id());
            let edit = match place {
                Place::Insert | Place::Append => ops::insert(&self.project, track, clip, split_id),
                Place::Overwrite => ops::overwrite(&self.project, track, clip, split_id),
            };
            match edit {
                Ok(e) => edits.push(e),
                Err(e) => {
                    self.status = Some(e.to_string());
                    return;
                }
            }
        }
        if self.apply(Edit::Batch(edits)) {
            self.active = Monitor::Program;
            // Insertar y sobrescribir desde el visor dejan el cabezal al final de lo colocado;
            // soltar un clip con el ratón no lo mueve.
            if move_playhead {
                self.program.seek(at + duration);
            }
        }
    }

    /// Corta en `at` las pistas dadas (las que no tengan clip ahí se saltan).
    fn cut(&mut self, tracks: impl IntoIterator<Item = usize>, at: Frames) {
        let mut edits = Vec::new();
        for track in tracks {
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

    /// Operación sobre el clip seleccionado (y, con la selección enlazada, su pareja).
    fn on_selected(&mut self, f: impl Fn(&Project, usize, ClipId) -> Result<Edit, EditError>) {
        if self.timeline.selection(&self.project).is_empty() {
            self.status = Some("Selecciona un clip en la línea de tiempo.".into());
            return;
        }
        match self.timeline.on_selection(&self.project, f) {
            Ok(e) => {
                self.apply(e);
            }
            Err(e) => self.status = Some(e.to_string()),
        }
    }

    /// Salta al punto de corte anterior o siguiente (↑ / ↓).
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
        if let Some(dir) = rfd::FileDialog::new().set_title("Abrir proyecto (carpeta .cinder)").pick_folder() {
            self.load_project(&dir);
        }
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
                self.source.pause(self.program.mixer());
                self.source = Viewer::new();
                self.timeline.selected = None;
                self.fix_dests();
                self.active = Monitor::Program;
                self.status = Some(format!("Abierto {}", dir.display()));
            }
            Err(e) => self.status = Some(e.to_string()),
        }
    }

    fn handle_pool(&mut self, out: PoolOutput) {
        if out.import {
            let paths = Viewer::pick_paths();
            self.import(&paths);
        }
        if out.modified {
            self.modified = true;
        }
        if out.removed.is_some() {
            self.source.pause(self.program.mixer());
            self.source = Viewer::new();
            self.program.forget_media();
        }
        if let Some(e) = out.error {
            self.status = Some(e);
        }
        if let Some(id) = out.open {
            self.open_in_source(id);
        }
    }

    fn handle_timeline(&mut self, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Focus => {
                    self.active = Monitor::Program;
                    self.source.pause(self.program.mixer());
                }
                Action::Seek(f) => self.program.seek(f),
                Action::Apply(edit) => {
                    self.apply(edit);
                }
                Action::Error(e) => self.status = Some(e.to_string()),
                Action::Blade { track, id, at } => {
                    let tracks: Vec<usize> = if self.timeline.linked {
                        ops::linked(&self.project, track, id).into_iter().map(|(t, _)| t).collect()
                    } else {
                        vec![track]
                    };
                    self.cut(tracks, at);
                }
                Action::AddTrack(kind) => {
                    let track = Track::new(kind, self.project.timeline.next_track_name(kind));
                    self.apply(Edit::InsertTrack { index: self.project.timeline.tracks.len(), track });
                }
                Action::RemoveTrack(index) => {
                    self.timeline.selected = None;
                    self.apply(Edit::RemoveTrack { index });
                }
                Action::ToggleMute(index) => {
                    if let Some(t) = self.project.timeline.tracks.get_mut(index) {
                        t.muted = !t.muted;
                        self.modified = true;
                        self.program.invalidate();
                    }
                }
                Action::SetDest(index) => match self.project.timeline.tracks.get(index).map(|t| t.kind) {
                    Some(TrackKind::Video) => self.dest_video = index,
                    Some(TrackKind::Audio) => self.dest_audio = index,
                    None => {}
                },
                Action::DropMedia { media, at } => {
                    let len = self.project.media.iter().find(|m| m.id == media).map_or(0, |m| m.duration);
                    self.place_media(media, 0, len, at, Place::Overwrite, false);
                }
            }
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

        let none = Modifiers::NONE;
        let cmd = Modifiers::COMMAND;
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        if pressed(ctx, Key::I, cmd) {
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
        if pressed(ctx, Key::A, none) {
            self.timeline.tool = Tool::Select;
        }
        if pressed(ctx, Key::B, none) {
            self.timeline.tool = Tool::Blade;
        }
        if pressed(ctx, Key::N, none) {
            self.timeline.snapping = !self.timeline.snapping;
        }
        if pressed(ctx, Key::L, cmd_shift) {
            self.timeline.linked = !self.timeline.linked;
        }
        if pressed(ctx, Key::F9, none) {
            self.place_from_source(Place::Insert);
        }
        if pressed(ctx, Key::F10, none) {
            self.place_from_source(Place::Overwrite);
        }
        if pressed(ctx, Key::F12, Modifiers::SHIFT) {
            self.place_from_source(Place::Append);
        }
        if pressed(ctx, Key::B, cmd) {
            let all = 0..self.project.timeline.tracks.len();
            self.cut(all, self.program.playhead);
        }
        if pressed(ctx, Key::Backspace, none) {
            self.on_selected(|_, track, id| Ok(Edit::RemoveClip { track, id }));
        }
        if pressed(ctx, Key::Delete, none) || pressed(ctx, Key::Backspace, Modifiers::SHIFT) {
            self.on_selected(ops::ripple_delete);
        }
        // , y . empujan el clip un fotograma; con Alt, slip; con Mayús, slide; con Ctrl, roll
        // del corte de su derecha.
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
            self.timeline.fit(self.project.timeline.duration());
        }
    }

    fn has_viewer(&self) -> bool {
        matches!(self.page, Page::Medios | Page::Edicion)
    }

    /// Barra superior: paneles a la izquierda, el proyecto en el centro y archivo a la derecha.
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let rect = ui.available_rect_before_wrap();
        let title =
            if self.modified { format!("{}  ·  Editado", self.project.name) } else { self.project.name.clone() };
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, title, FontId::proportional(12.5), theme::TEXT);
        ui.horizontal_centered(|ui| {
            ui.add_space(8.0);
            if self.page == Page::Edicion {
                let color = if self.show_pool { theme::TEXT_BRIGHT } else { theme::TEXT_DIM };
                let button = egui::Button::selectable(self.show_pool, RichText::new("Medios").color(color));
                if ui.add(button).on_hover_text("Mostrar u ocultar el panel de medios").clicked() {
                    self.show_pool = !self.show_pool;
                }
            }
            if let Some(msg) = &self.status {
                ui.add_space(6.0);
                ui.label(RichText::new(msg).color(theme::WARN));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                let s = &self.project.settings;
                let format = format!("{}×{} · {:.3} fps", s.width, s.height, s.fps.as_f64());
                ui.label(RichText::new(format).color(theme::TEXT_DIM));
                ui.add_space(6.0);
                if ui.button("Guardar").on_hover_text("Ctrl+S · con Mayús, guardar como").clicked() {
                    self.save(false);
                }
                if ui.button("Abrir").on_hover_text("Abrir un proyecto (Ctrl+O)").clicked() {
                    self.open_project();
                }
            });
        });
    }

    /// Barra inferior con las siete páginas: icono encima del nombre.
    fn page_bar(&mut self, ui: &mut egui::Ui) {
        let rect = ui.available_rect_before_wrap();
        let painter = ui.painter().clone();
        painter.text(
            rect.left_center() + Vec2::new(14.0, 0.0),
            Align2::LEFT_CENTER,
            "Cinder Studio",
            FontId::proportional(13.0),
            theme::BRAND,
        );
        painter.text(
            rect.right_center() + Vec2::new(-14.0, 0.0),
            Align2::RIGHT_CENTER,
            concat!("v", env!("CARGO_PKG_VERSION")),
            FontId::proportional(11.0),
            theme::TEXT_DIM,
        );
        let cell = Vec2::new(92.0, rect.height());
        let mut x = rect.center().x - cell.x * Page::ALL.len() as f32 / 2.0;
        for page in Page::ALL {
            let r = Rect::from_min_size(Pos2::new(x, rect.top()), cell);
            let resp = ui.allocate_rect(r, Sense::click()).on_hover_text(page.shortcut_label());
            let active = self.page == page;
            let color = if active {
                theme::TEXT_BRIGHT
            } else if resp.hovered() {
                theme::TEXT
            } else {
                theme::TEXT_DIM
            };
            if active {
                painter.rect_filled(Rect::from_min_size(r.min, Vec2::new(r.width(), 2.0)), 0.0, theme::HOT);
            }
            let icon = Rect::from_center_size(Pos2::new(r.center().x, r.top() + 19.0), Vec2::splat(17.0));
            icons::paint(&painter, icon, page_icon(page), color, false);
            painter.text(
                Pos2::new(r.center().x, r.bottom() - 13.0),
                Align2::CENTER_CENTER,
                page.label(),
                FontId::proportional(11.0),
                color,
            );
            if resp.clicked() {
                self.page = page;
            }
            x += cell.x;
        }
    }

    /// Barra entre los visores y la línea de tiempo: herramientas, colocar y zoom.
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            ui.add_space(10.0);
            let tl = &mut self.timeline;
            if icons::button(ui, Icon::Arrow, tl.tool == Tool::Select).on_hover_text("Selección (A)").clicked() {
                tl.tool = Tool::Select;
            }
            if icons::button(ui, Icon::Blade, tl.tool == Tool::Blade).on_hover_text("Cuchilla (B)").clicked() {
                tl.tool = Tool::Blade;
            }
            ui.separator();
            let mut place = None;
            if icons::button(ui, Icon::Insert, false).on_hover_text("Insertar en el cabezal (F9)").clicked() {
                place = Some(Place::Insert);
            }
            if icons::button(ui, Icon::Overwrite, false).on_hover_text("Sobrescribir en el cabezal (F10)").clicked() {
                place = Some(Place::Overwrite);
            }
            if icons::button(ui, Icon::Append, false).on_hover_text("Añadir al final (Mayús+F12)").clicked() {
                place = Some(Place::Append);
            }
            ui.separator();
            if icons::button(ui, Icon::Magnet, tl.snapping).on_hover_text("Imán (N)").clicked() {
                tl.snapping = !tl.snapping;
            }
            if icons::button(ui, Icon::Link, tl.linked).on_hover_text("Selección enlazada (Ctrl+Mayús+L)").clicked() {
                tl.linked = !tl.linked;
            }
            let mut fit = false;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                if icons::button(ui, Icon::Fit, false).on_hover_text("Encajar todo (Mayús+Z)").clicked() {
                    fit = true;
                }
                if icons::button(ui, Icon::Plus, false).on_hover_text("Acercar (Ctrl++)").clicked() {
                    tl.zoom_by(1.5);
                }
                ui.spacing_mut().slider_width = 110.0;
                ui.add(egui::Slider::new(&mut tl.zoom, 0.05..=40.0).logarithmic(true).show_value(false));
                if icons::button(ui, Icon::Minus, false).on_hover_text("Alejar (Ctrl+-)").clicked() {
                    tl.zoom_by(1.0 / 1.5);
                }
            });
            if fit {
                self.timeline.fit(self.project.timeline.duration());
            }
            if let Some(p) = place {
                self.place_from_source(p);
            }
        });
    }

    fn medios_page(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("metadatos")
            .resizable(true)
            .default_size(300.0)
            .min_size(220.0)
            .frame(theme::flush(theme::PANEL))
            .show(ui, |ui| self.pool.metadata_ui(ui, &self.project));
        let mut pool_out = PoolOutput::default();
        let h = ui.available_height();
        egui::Panel::bottom("medios-grande")
            .resizable(true)
            .default_size(h * 0.45)
            .min_size(160.0)
            .frame(theme::flush(theme::PANEL))
            .show(ui, |ui| pool_out = self.pool.ui(ui, &mut self.project));
        egui::CentralPanel::default().frame(theme::flush(theme::PANEL)).show(ui, |ui| {
            if self.source.ui(ui, true, self.program.mixer()) {
                self.active = Monitor::Source;
            }
        });
        self.handle_pool(pool_out);
    }

    fn edicion_page(&mut self, ui: &mut egui::Ui) {
        let mut actions = Vec::new();
        egui::Panel::bottom("linea-de-tiempo")
            .resizable(true)
            .default_size(310.0)
            .min_size(170.0)
            .frame(theme::flush(theme::BG))
            .show(ui, |ui| {
                let view = timeline_view::View {
                    project: &self.project,
                    pool: &self.pool,
                    playhead: self.program.playhead,
                    playing: self.program.is_playing(),
                    dest_video: self.dest_video,
                    dest_audio: self.dest_audio,
                };
                actions = self.timeline.ui(ui, &view);
            });
        egui::Panel::bottom("herramientas")
            .exact_size(32.0)
            .frame(theme::flush(theme::PANEL))
            .show(ui, |ui| self.toolbar(ui));
        let mut pool_out = PoolOutput::default();
        if self.show_pool {
            egui::Panel::left("panel-medios")
                .resizable(true)
                .default_size(400.0)
                .min_size(260.0)
                .frame(theme::flush(theme::PANEL))
                .show(ui, |ui| pool_out = self.pool.ui(ui, &mut self.project));
        }
        egui::CentralPanel::default().frame(theme::flush(theme::LINE)).show(ui, |ui| {
            let full = ui.available_rect_before_wrap();
            let mid = full.center().x;
            let left = Rect::from_min_max(full.min, Pos2::new(mid - 0.5, full.bottom()));
            let right = Rect::from_min_max(Pos2::new(mid + 0.5, full.top()), full.max);
            let source_active = self.active == Monitor::Source;
            ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
                if self.source.ui(ui, source_active, self.program.mixer()) {
                    self.active = Monitor::Source;
                }
            });
            ui.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| {
                if self.program.ui(ui, &self.project, !source_active) {
                    self.active = Monitor::Program;
                }
            });
        });
        self.handle_pool(pool_out);
        self.handle_timeline(actions);
    }

    fn page_body(&mut self, ui: &mut egui::Ui) {
        match self.page {
            Page::Medios => self.medios_page(ui),
            Page::Edicion => self.edicion_page(ui),
            other => {
                egui::CentralPanel::default().frame(theme::flush(theme::BG)).show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(ui.available_height() * 0.34);
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
                        icons::paint(ui.painter(), rect, page_icon(other), theme::TEXT_DIM, false);
                        ui.add_space(10.0);
                        ui.label(RichText::new(other.label()).size(22.0).color(theme::TEXT_BRIGHT));
                        ui.add_space(6.0);
                        ui.label(other.summary());
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(format!("Esta página llega en la fase {} del roadmap.", other.phase()))
                                .color(theme::TEXT_DIM),
                        );
                    });
                });
            }
        }
    }
}

fn page_icon(page: Page) -> Icon {
    match page {
        Page::Medios => Icon::Film,
        Page::Corte => Icon::Scissors,
        Page::Edicion => Icon::Tracks,
        Page::Composicion => Icon::Nodes,
        Page::Color => Icon::Wheel,
        Page::Audio => Icon::Wave,
        Page::Entrega => Icon::Export,
    }
}

impl eframe::App for CinderApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_shortcuts(ctx);
        if self.has_viewer() {
            match (self.page, self.active) {
                (Page::Edicion, Monitor::Program) => self.program.handle_input(ctx, &self.project),
                _ => self.source.handle_input(ctx, self.program.mixer()),
            }
        }
        // Solo suena un visor a la vez: el activo.
        match self.active {
            Monitor::Source if self.program.is_playing() => self.program.pause(),
            Monitor::Program if self.source.is_playing() => self.source.pause(self.program.mixer()),
            _ => {}
        }
        if self.page != Page::Edicion && self.program.is_playing() {
            self.program.pause();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("barra-superior")
            .exact_size(30.0)
            .frame(theme::flush(theme::PANEL))
            .show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("barra-paginas")
            .exact_size(52.0)
            .frame(theme::flush(theme::PANEL))
            .show(ui, |ui| self.page_bar(ui));
        self.page_body(ui);
    }
}
