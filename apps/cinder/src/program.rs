//! Visor de programa: enseña la línea de tiempo en el cabezal y la reproduce.
//!
//! Se ve el clip de la pista de vídeo más alta que haya en el cabezal; en un hueco, negro. Cada
//! medio tiene su reproductor (se abren al hacer falta y se quedan abiertos); al pasar de un
//! clip al siguiente se salta en el nuevo reproductor y se para el anterior.
//!
//! En esta etapa el programa suena mudo: la mezcla de las pistas de audio llega en el siguiente
//! paso de la Fase 2. El visor de origen sí suena.

use std::collections::HashMap;
use std::time::Instant;

use cinder_core::{Clip, Frames, MediaId, Project, TrackKind};
use cinder_media::Player;
use eframe::egui::{self, Color32, TextureHandle, Vec2};

use crate::media_pool::frames_to_secs;
use crate::viewer::{draw_picture, play_button, timecode, upload};

pub struct Program {
    players: HashMap<MediaId, Player>,
    errors: HashMap<MediaId, String>,
    texture: Option<TextureHandle>,
    shown: Option<(MediaId, f64)>,
    pub playhead: Frames,
    playing: bool,
    /// Clip que suena ahora (id y medio) y desde qué fotograma se cuenta en un hueco.
    current: Option<Clip>,
    gap_from: Option<(Instant, Frames)>,
    /// El cabezal cambió desde fuera (clic en la regla, flechas): hay que saltar.
    dirty: bool,
}

/// El clip visible en `at`: el de la pista de vídeo más alta.
fn top_clip(project: &Project, at: Frames) -> Option<Clip> {
    project.timeline.tracks.iter().rev().filter(|t| t.kind == TrackKind::Video).find_map(|t| t.clip_at(at).cloned())
}

impl Program {
    pub fn new() -> Self {
        Self {
            players: HashMap::new(),
            errors: HashMap::new(),
            texture: None,
            shown: None,
            playhead: 0,
            playing: false,
            current: None,
            gap_from: None,
            dirty: true,
        }
    }

    /// Mueve el cabezal (y para la reproducción, como al hacer clic en la regla).
    pub fn seek(&mut self, at: Frames) {
        self.pause();
        self.playhead = at.max(0);
        self.dirty = true;
    }

    /// La línea de tiempo cambió: lo que hay en el cabezal puede ser otro clip.
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    pub fn play(&mut self, project: &Project) {
        if self.playing {
            return;
        }
        if self.playhead >= project.timeline.duration() {
            self.playhead = 0;
        }
        self.playing = true;
        self.current = None;
        self.gap_from = None;
    }

    pub fn pause(&mut self) {
        self.playing = false;
        self.gap_from = None;
        for p in self.players.values_mut() {
            p.pause();
        }
    }

    pub fn toggle(&mut self, project: &Project) {
        if self.playing { self.pause() } else { self.play(project) }
    }

    pub fn forget_media(&mut self) {
        self.players.clear();
        self.errors.clear();
        self.texture = None;
        self.shown = None;
        self.current = None;
        self.dirty = true;
    }

    fn player(&mut self, project: &Project, media: MediaId) -> Option<&mut Player> {
        if !self.players.contains_key(&media) && !self.errors.contains_key(&media) {
            let item = project.media.iter().find(|m| m.id == media)?;
            match Player::open_silent(&item.path) {
                Ok(p) => {
                    self.players.insert(media, p);
                }
                Err(e) => {
                    self.errors.insert(media, e.to_string());
                }
            }
        }
        self.players.get_mut(&media)
    }

    /// Segundo del medio que corresponde al fotograma `at` dentro del clip.
    fn source_secs(project: &Project, clip: &Clip, at: Frames) -> f64 {
        frames_to_secs(clip.source_in + (at - clip.start), project.settings.fps)
    }

    /// Pone el reproductor del clip en el punto del cabezal (y en marcha si se está reproduciendo).
    fn enter(&mut self, project: &Project, clip: Option<Clip>) {
        if let Some(old) = self.current.take()
            && let Some(p) = self.players.get_mut(&old.media)
        {
            p.pause();
        }
        self.gap_from = None;
        match clip {
            Some(c) => {
                let secs = Self::source_secs(project, &c, self.playhead);
                let playing = self.playing;
                if let Some(p) = self.player(project, c.media) {
                    p.seek(secs);
                    if playing {
                        p.play();
                    }
                }
                self.current = Some(c);
            }
            None => {
                if self.playing {
                    self.gap_from = Some((Instant::now(), self.playhead));
                }
                self.texture = None;
                self.shown = None;
            }
        }
    }

    /// Avanza el cabezal y trae el fotograma que toca. Llamar en cada repintado.
    pub fn update(&mut self, ctx: &egui::Context, project: &Project) {
        let fps = project.settings.fps;
        let end = project.timeline.duration();
        if self.dirty {
            self.dirty = false;
            let clip = top_clip(project, self.playhead);
            self.enter(project, clip);
        }
        if self.playing {
            let next = match (&self.current, self.gap_from) {
                (Some(c), _) => {
                    let pos = self.players.get(&c.media).map(Player::position);
                    match pos {
                        Some(secs) => (secs * fps.as_f64()).round() as Frames - c.source_in + c.start,
                        None => c.end(),
                    }
                }
                (None, Some((since, from))) => from + (since.elapsed().as_secs_f64() * fps.as_f64()) as Frames,
                (None, None) => self.playhead,
            };
            self.playhead = next.max(self.playhead);
            if self.playhead >= end {
                self.playhead = end;
                self.pause();
            } else {
                // ¿Cambió lo que hay debajo del cabezal? (fin de clip, empieza otro, se acaba un hueco)
                let below = top_clip(project, self.playhead);
                if below.as_ref().map(|c| c.id) != self.current.as_ref().map(|c| c.id) {
                    self.enter(project, below);
                }
            }
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        if let Some(c) = self.current.clone()
            && let Some(p) = self.players.get_mut(&c.media)
            && let Some(frame) = p.frame()
            && self.shown != Some((c.media, frame.pts))
        {
            upload(ctx, &mut self.texture, "visor-programa", frame);
            self.shown = Some((c.media, frame.pts));
        }
    }

    /// Devuelve `true` si se hizo clic en la imagen.
    pub fn ui(&mut self, ui: &mut egui::Ui, project: &Project) -> bool {
        self.update(ui.ctx(), project);
        let fps = project.settings.fps.as_f64();
        let area = ui.available_size() - Vec2::new(0.0, 40.0);
        let clicked = draw_picture(ui, self.texture.as_ref(), area).clicked();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if play_button(ui, self.playing).on_hover_text("Espacio · K / L").clicked() {
                self.toggle(project);
            }
            ui.monospace(timecode(self.playhead as f64 / fps, fps));
            ui.weak("/");
            ui.monospace(timecode(project.timeline.duration() as f64 / fps, fps));
            ui.weak("Programa (sin sonido todavía)");
            if let Some(c) = &self.current
                && let Some(e) = self.errors.get(&c.media)
            {
                ui.colored_label(Color32::from_rgb(0xF0, 0x7F, 0x7F), e);
            }
        });
        clicked
    }

    /// Atajos del visor de programa, con él activo.
    pub fn handle_input(&mut self, ctx: &egui::Context, project: &Project) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (space, k, l, j, left, right, home, end) = ctx.input(|i| {
            if i.modifiers.any() {
                return Default::default();
            }
            (
                i.key_pressed(egui::Key::Space),
                i.key_pressed(egui::Key::K),
                i.key_pressed(egui::Key::L),
                i.key_pressed(egui::Key::J),
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::Home),
                i.key_pressed(egui::Key::End),
            )
        });
        let fps = project.settings.fps.as_f64().round() as Frames;
        if space {
            self.toggle(project);
        }
        if k {
            self.pause();
        }
        if l {
            self.play(project);
        }
        if j {
            self.seek(self.playhead - fps);
        }
        if left {
            self.seek(self.playhead - 1);
        }
        if right {
            self.seek(self.playhead + 1);
        }
        if home {
            self.seek(0);
        }
        if end {
            self.seek(project.timeline.duration());
        }
    }
}
