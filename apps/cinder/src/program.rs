//! Visor de programa: enseña la línea de tiempo en el cabezal y la reproduce con su sonido.
//!
//! El reloj es el del mezclador de audio ([`cinder_media::Mixer`]): el cabezal va donde diga
//! el sonido. Imagen: el clip de la pista de vídeo visible más alta; cada medio tiene su
//! reproductor mudo, que se coloca al entrar en cada clip y se recoloca si se desvía. Sonido:
//! cada clip de las pistas de audio sin silenciar se programa en el mezclador un poco antes de
//! que le toque, en su muestra exacta.

use std::collections::HashMap;
use std::time::Instant;

use cinder_core::{Clip, ClipId, Frames, MediaId, Project, TrackKind};
use cinder_media::{AudioClipHandle, Mixer, Player};
use eframe::egui::{self, TextureHandle};

use crate::media_pool::frames_to_secs;
use crate::monitor::{self, MonitorInput, timecode};
use crate::theme;

/// Con cuánta antelación (en segundos) se programa el audio de un clip.
const LOOKAHEAD: f64 = 1.0;
/// Desvío entre la imagen y el reloj a partir del cual se recoloca el reproductor.
const MAX_DRIFT: f64 = 0.08;

pub struct Program {
    mixer: Mixer,
    players: HashMap<MediaId, Player>,
    errors: HashMap<MediaId, String>,
    /// Clips de audio ya programados, con el fotograma en que acaban.
    voices: HashMap<ClipId, (Frames, AudioClipHandle)>,
    texture: Option<TextureHandle>,
    shown: Option<(MediaId, f64)>,
    pub playhead: Frames,
    playing: bool,
    /// Segundo de la línea de tiempo y valor del reloj del mezclador al empezar a reproducir.
    base_secs: f64,
    base_clock: u64,
    /// Clip que se está viendo.
    current: Option<Clip>,
    last_resync: Option<Instant>,
    /// El cabezal o la línea de tiempo cambiaron desde fuera: hay que recolocar la imagen.
    dirty: bool,
}

/// El clip visible en `at`: el de la pista de vídeo sin ocultar más alta.
fn top_clip(project: &Project, at: Frames) -> Option<Clip> {
    project
        .timeline
        .tracks
        .iter()
        .rev()
        .filter(|t| t.kind == TrackKind::Video && !t.muted)
        .find_map(|t| t.clip_at(at).cloned())
}

impl Program {
    pub fn new() -> Self {
        Self {
            mixer: Mixer::new(),
            players: HashMap::new(),
            errors: HashMap::new(),
            voices: HashMap::new(),
            texture: None,
            shown: None,
            playhead: 0,
            playing: false,
            base_secs: 0.0,
            base_clock: 0,
            current: None,
            last_resync: None,
            dirty: true,
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// El mezclador de la aplicación: el visor de origen también suena por él.
    pub fn mixer(&mut self) -> &mut Mixer {
        &mut self.mixer
    }

    /// Mueve el cabezal (y para la reproducción, como al hacer clic en la regla).
    pub fn seek(&mut self, at: Frames) {
        self.pause();
        self.playhead = at.max(0);
        self.dirty = true;
    }

    /// La línea de tiempo cambió: lo que hay en el cabezal puede ser otro clip y el audio
    /// programado ya no vale.
    pub fn invalidate(&mut self) {
        self.dirty = true;
        self.voices.clear();
    }

    pub fn play(&mut self, project: &Project) {
        if self.playing {
            return;
        }
        if self.playhead >= project.timeline.duration() {
            self.playhead = 0;
        }
        self.playing = true;
        self.base_secs = frames_to_secs(self.playhead, project.settings.fps);
        self.base_clock = self.mixer.clock();
        self.voices.clear();
        self.dirty = true;
        self.mixer.set_playing(true);
    }

    pub fn pause(&mut self) {
        // Solo para el mezclador si era el programa el que sonaba (puede ser el visor de origen).
        if std::mem::take(&mut self.playing) {
            self.mixer.set_playing(false);
        }
        self.voices.clear();
        for p in self.players.values_mut() {
            p.pause();
        }
    }

    pub fn toggle(&mut self, project: &Project) {
        if self.playing { self.pause() } else { self.play(project) }
    }

    /// Se abrió otro proyecto: fuera reproductores y texturas.
    pub fn forget_media(&mut self) {
        self.pause();
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

    /// Segundo de la línea de tiempo según el reloj del mezclador.
    fn now_secs(&self) -> f64 {
        let elapsed = self.mixer.clock().saturating_sub(self.base_clock);
        self.base_secs + elapsed as f64 / f64::from(self.mixer.rate().max(1))
    }

    /// Coloca el reproductor del clip en el segundo `secs` de la línea de tiempo.
    fn enter(&mut self, project: &Project, clip: Option<Clip>, secs: f64) {
        if let Some(old) = self.current.take()
            && let Some(p) = self.players.get_mut(&old.media)
        {
            p.pause();
        }
        self.last_resync = Some(Instant::now());
        match clip {
            Some(c) => {
                let fps = project.settings.fps;
                let source = frames_to_secs(c.source_in, fps) + (secs - frames_to_secs(c.start, fps)).max(0.0);
                let playing = self.playing;
                if let Some(p) = self.player(project, c.media) {
                    p.seek(source);
                    if playing {
                        p.play();
                    }
                }
                self.current = Some(c);
            }
            None => {
                self.texture = None;
                self.shown = None;
            }
        }
    }

    /// Programa en el mezclador los clips de audio que suenan ahora o están a punto.
    fn schedule_audio(&mut self, project: &Project, now: f64) {
        self.mixer.reap();
        if !self.mixer.has_output() {
            return;
        }
        let fps = project.settings.fps;
        let rate = f64::from(self.mixer.rate());
        let playhead = self.playhead;
        self.voices.retain(|_, (end, _)| *end + fps.as_f64() as Frames / 4 > playhead);
        for track in project.timeline.tracks.iter().filter(|t| t.kind == TrackKind::Audio && !t.muted) {
            for c in track.clips() {
                let (start, end) = (frames_to_secs(c.start, fps), frames_to_secs(c.end(), fps));
                if end <= now || start > now + LOOKAHEAD || self.voices.contains_key(&c.id) {
                    continue;
                }
                let Some(item) = project.media.iter().find(|m| m.id == c.media) else { continue };
                // Si el clip ya había empezado, entra por donde va; si no, desde su principio.
                let from = start.max(now);
                let source = frames_to_secs(c.source_in, fps) + (from - start);
                let frames = ((end - from) * rate) as u64;
                let at_clock = self.base_clock + ((from - self.base_secs).max(0.0) * rate) as u64;
                if let Some(handle) = self.mixer.add_clip(&item.path, source, frames, at_clock, 1.0) {
                    self.voices.insert(c.id, (c.end(), handle));
                }
            }
        }
    }

    /// Avanza el cabezal y trae el fotograma que toca. Llamar en cada repintado.
    pub fn update(&mut self, ctx: &egui::Context, project: &Project) {
        let fps = project.settings.fps;
        let end = project.timeline.duration();
        let mut now = frames_to_secs(self.playhead, fps);
        if self.playing {
            now = self.now_secs();
            self.playhead = (now * fps.as_f64()).floor() as Frames;
            if self.playhead >= end {
                self.playhead = end;
                self.pause();
            }
        }
        let below = top_clip(project, self.playhead);
        if self.dirty || below.as_ref().map(|c| c.id) != self.current.as_ref().map(|c| c.id) {
            self.dirty = false;
            self.enter(project, below, now);
        }
        if self.playing {
            self.schedule_audio(project, now);
            // El reproductor lleva su propio reloj: si se separa del sonido, se recoloca.
            if let Some(c) = &self.current
                && let Some(p) = self.players.get_mut(&c.media)
            {
                let want = frames_to_secs(c.source_in, fps) + (now - frames_to_secs(c.start, fps));
                let settled = self.last_resync.is_none_or(|t| t.elapsed().as_secs_f64() > 0.5);
                if settled && (p.position() - want).abs() > MAX_DRIFT {
                    p.seek(want);
                    self.last_resync = Some(Instant::now());
                }
            }
            ctx.request_repaint();
        } else {
            self.mixer.reap();
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        if let Some(c) = self.current.clone()
            && let Some(p) = self.players.get_mut(&c.media)
            && let Some(frame) = p.frame()
            && self.shown != Some((c.media, frame.pts))
        {
            monitor::upload(ctx, &mut self.texture, "visor-programa", frame);
            self.shown = Some((c.media, frame.pts));
        }
    }

    /// Devuelve `true` si se tocó el visor.
    pub fn ui(&mut self, ui: &mut egui::Ui, project: &Project, active: bool) -> bool {
        self.update(ui.ctx(), project);
        let fps = project.settings.fps.as_f64();
        let end = project.timeline.duration();
        let error = self.current.as_ref().and_then(|c| self.errors.get(&c.media)).cloned();
        let note = match (&error, self.mixer.error()) {
            (Some(e), _) => Some((e.as_str(), theme::ERROR)),
            (None, Some(_)) => Some(("sin tarjeta de sonido", theme::WARN)),
            _ => None,
        };
        let out = monitor::show(
            ui,
            MonitorInput {
                kind: "Programa",
                title: &project.name,
                timecode: timecode(self.playhead as f64 / fps, fps),
                duration: timecode(end as f64 / fps, fps),
                texture: self.texture.as_ref(),
                playing: self.playing,
                position: if end > 0 { self.playhead as f64 / end as f64 } else { 0.0 },
                marks: None,
                active,
                with_marks: false,
                empty: (end == 0).then_some("La línea de tiempo está vacía"),
                note,
            },
        );
        if out.toggle {
            self.toggle(project);
        }
        if out.to_start {
            self.seek(0);
        }
        if out.to_end {
            self.seek(end);
        }
        if out.step_back {
            self.seek(self.playhead - 1);
        }
        if out.step_fwd {
            self.seek(self.playhead + 1);
        }
        if let Some(t) = out.scrub {
            self.seek((t * end as f64).round() as Frames);
        }
        out.clicked
    }

    /// Atajos del visor de programa, con él activo.
    pub fn handle_input(&mut self, ctx: &egui::Context, project: &Project) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let key = |k: egui::Key| ctx.input(|i| !i.modifiers.any() && i.key_pressed(k));
        let second = project.settings.fps.as_f64().round() as Frames;
        if key(egui::Key::Space) {
            self.toggle(project);
        }
        if key(egui::Key::K) {
            self.pause();
        }
        if key(egui::Key::L) {
            self.play(project);
        }
        if key(egui::Key::J) {
            self.seek(self.playhead - second);
        }
        if key(egui::Key::ArrowLeft) {
            self.seek(self.playhead - 1);
        }
        if key(egui::Key::ArrowRight) {
            self.seek(self.playhead + 1);
        }
        if key(egui::Key::Home) {
            self.seek(0);
        }
        if key(egui::Key::End) {
            self.seek(project.timeline.duration());
        }
    }
}
