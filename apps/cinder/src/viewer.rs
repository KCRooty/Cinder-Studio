//! Visor de origen: un clip suelto con sus marcas de entrada y salida.
//!
//! La imagen la lleva un reproductor mudo y el sonido sale por el mezclador de la aplicación
//! (el mismo del programa): así hay una sola salida de audio abierta, que es lo único que
//! admiten algunos sistemas. El reloj es el del mezclador; si la imagen se separa, se recoloca.
//!
//! Atajos (con el visor activo y sin un campo de texto activo):
//! Espacio reproduce o para · K para · L reproduce · J retrocede 1 s · ← / → un fotograma ·
//! Inicio / Fin · I / O marcan entrada y salida · Alt+X las quita.

use std::path::{Path, PathBuf};

use cinder_media::{AudioClipHandle, MediaInfo, Mixer, Player};
use eframe::egui::{self, Key, TextureHandle};

use crate::monitor::{self, MonitorInput, timecode};
use crate::theme;

pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "mkv", "webm", "avi", "m4v", "mxf", "mts", "ts"];

pub struct Viewer {
    player: Option<Player>,
    texture: Option<TextureHandle>,
    /// pts del fotograma que está en la textura, para no subirlo otra vez.
    shown_pts: Option<f64>,
    error: Option<String>,
    /// Entrada y salida marcadas, en segundos del clip.
    mark_in: Option<f64>,
    mark_out: Option<f64>,
    /// El audio del clip sonando en el mezclador.
    voice: Option<AudioClipHandle>,
    /// Segundo del clip y valor del reloj del mezclador al empezar a reproducir.
    base: Option<(f64, u64)>,
    last_resync: Option<std::time::Instant>,
}

impl Viewer {
    pub fn new() -> Self {
        Self {
            player: None,
            texture: None,
            shown_pts: None,
            error: None,
            mark_in: None,
            mark_out: None,
            voice: None,
            base: None,
            last_resync: None,
        }
    }

    pub fn open(&mut self, path: &Path) {
        // Primero se suelta el reproductor viejo: su hilo y su audio se cierran antes.
        self.player = None;
        self.texture = None;
        self.shown_pts = None;
        self.mark_in = None;
        self.mark_out = None;
        self.voice = None;
        self.base = None;
        match Player::open_silent(path) {
            Ok(p) => {
                self.error = None;
                self.player = Some(p);
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.player.as_ref().map(|p| p.info().path.as_path())
    }

    pub fn info(&self) -> Option<&MediaInfo> {
        self.player.as_ref().map(Player::info)
    }

    pub fn is_playing(&self) -> bool {
        self.base.is_some()
    }

    /// Segundo del clip: reproduciendo lo dice el reloj del mezclador; en pausa, el reproductor.
    fn position(&self, mixer: &Mixer) -> f64 {
        let Some(p) = self.player.as_ref() else { return 0.0 };
        match self.base {
            Some((secs, clock)) => {
                let elapsed = mixer.clock().saturating_sub(clock) as f64 / f64::from(mixer.rate().max(1));
                (secs + elapsed).min(p.info().duration.max(0.0))
            }
            None => p.position(),
        }
    }

    pub fn play(&mut self, mixer: &mut Mixer) {
        let Some(p) = self.player.as_mut() else { return };
        if self.base.is_some() {
            return;
        }
        let duration = p.info().duration;
        if duration > 0.0 && p.position() >= duration - 1e-3 {
            p.seek(0.0);
        }
        let from = p.position();
        p.play();
        mixer.set_playing(true);
        let clock = mixer.clock();
        self.base = Some((from, clock));
        self.last_resync = Some(std::time::Instant::now());
        if p.info().audio_codec.is_some() {
            let frames = ((duration - from).max(0.0) * f64::from(mixer.rate())) as u64;
            self.voice = mixer.add_clip(&p.info().path, from, frames, clock, 1.0);
        }
    }

    pub fn pause(&mut self, mixer: &mut Mixer) {
        if self.base.take().is_some() {
            mixer.set_playing(false);
        }
        self.voice = None;
        if let Some(p) = self.player.as_mut() {
            p.pause();
        }
    }

    pub fn toggle(&mut self, mixer: &mut Mixer) {
        if self.is_playing() { self.pause(mixer) } else { self.play(mixer) }
    }

    /// Salta a un segundo del clip; si estaba reproduciendo, sigue desde ahí.
    fn seek(&mut self, mixer: &mut Mixer, secs: f64) {
        let playing = self.is_playing();
        self.pause(mixer);
        if let Some(p) = self.player.as_mut() {
            p.seek(secs);
        }
        if playing {
            self.play(mixer);
        }
    }

    /// Tramo marcado en segundos: de la entrada (o el principio) a la salida (o el final).
    pub fn marked_range(&self) -> Option<(f64, f64)> {
        let info = self.info()?;
        let a = self.mark_in.unwrap_or(0.0);
        let b = self.mark_out.unwrap_or(info.duration);
        (b > a).then_some((a, b))
    }

    /// Rutas elegidas en el diálogo de importar.
    pub fn pick_paths() -> Vec<PathBuf> {
        rfd::FileDialog::new()
            .set_title("Importar clips")
            .add_filter("Vídeo", VIDEO_EXTENSIONS)
            .add_filter("Todos los archivos", &["*"])
            .pick_files()
            .unwrap_or_default()
    }

    fn step(&self) -> f64 {
        let fps = self.info().map_or(0.0, |i| i.fps);
        1.0 / if fps > 0.0 { fps } else { 25.0 }
    }

    fn set_in(&mut self, mixer: &Mixer) {
        if self.player.is_none() {
            return;
        }
        let t = self.position(mixer);
        self.mark_in = Some(t);
        if self.mark_out.is_some_and(|o| o <= t) {
            self.mark_out = None;
        }
    }

    fn set_out(&mut self, mixer: &Mixer) {
        if self.player.is_none() {
            return;
        }
        let t = self.position(mixer);
        self.mark_out = Some(t);
        if self.mark_in.is_some_and(|i| i >= t) {
            self.mark_in = None;
        }
    }

    fn step_by(&mut self, mixer: &mut Mixer, frames: f64) {
        let t = self.position(mixer) + frames * self.step();
        self.pause(mixer);
        self.seek(mixer, t);
    }

    fn go_to_end(&mut self, mixer: &mut Mixer) {
        let t = self.info().map_or(0.0, |i| i.duration) - self.step();
        self.pause(mixer);
        self.seek(mixer, t);
    }

    pub fn handle_input(&mut self, ctx: &egui::Context, mixer: &mut Mixer) {
        if ctx.egui_wants_keyboard_input() || self.player.is_none() {
            return;
        }
        let key = |k: Key| ctx.input(|i| !i.modifiers.any() && i.key_pressed(k));
        if ctx.input(|i| i.modifiers.alt && i.key_pressed(Key::X)) {
            self.mark_in = None;
            self.mark_out = None;
        }
        if key(Key::I) {
            self.set_in(mixer);
        }
        if key(Key::O) {
            self.set_out(mixer);
        }
        if key(Key::ArrowRight) {
            self.step_by(mixer, 1.0);
        }
        if key(Key::ArrowLeft) {
            self.step_by(mixer, -1.0);
        }
        if key(Key::End) {
            self.go_to_end(mixer);
        }
        if key(Key::Space) {
            self.toggle(mixer);
        }
        if key(Key::K) {
            self.pause(mixer);
        }
        if key(Key::L) {
            self.play(mixer);
        }
        if key(Key::J) {
            let t = self.position(mixer) - 1.0;
            self.seek(mixer, t);
        }
        if key(Key::Home) {
            self.seek(mixer, 0.0);
        }
    }

    /// Mantiene la imagen pegada al reloj del mezclador y para al llegar al final.
    fn follow_clock(&mut self, mixer: &mut Mixer) {
        if self.base.is_none() {
            return;
        }
        let want = self.position(mixer);
        let Some(p) = self.player.as_mut() else { return };
        let ended = !p.is_playing() || (p.info().duration > 0.0 && want >= p.info().duration - 1e-3);
        if ended {
            self.pause(mixer);
            return;
        }
        let settled = self.last_resync.is_none_or(|t| t.elapsed().as_secs_f64() > 0.5);
        if settled && (p.position() - want).abs() > 0.08 {
            p.seek(want);
            self.last_resync = Some(std::time::Instant::now());
        }
    }

    /// Devuelve `true` si se tocó el visor (para que pase a ser el activo).
    pub fn ui(&mut self, ui: &mut egui::Ui, active: bool, mixer: &mut Mixer) -> bool {
        self.upload_frame(ui.ctx());
        self.follow_clock(mixer);
        let position = self.position(mixer);
        let playing = self.is_playing();
        let (title, tc, dur, pos, marks, note) = match self.player.as_ref() {
            Some(p) => {
                let info = p.info();
                let total = info.duration.max(0.001);
                let name = info.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let marks = (self.mark_in.is_some() || self.mark_out.is_some())
                    .then(|| (self.mark_in.unwrap_or(0.0) / total, self.mark_out.unwrap_or(total) / total));
                let note = mixer.error().map(|_| "sin tarjeta de sonido".to_owned());
                (name, timecode(position, info.fps), timecode(info.duration, info.fps), position / total, marks, note)
            }
            None => (String::new(), "00:00:00:00".into(), "00:00:00:00".into(), 0.0, None, self.error.clone()),
        };
        let out = monitor::show(
            ui,
            MonitorInput {
                kind: "Origen",
                title: &title,
                timecode: tc,
                duration: dur,
                texture: self.texture.as_ref(),
                playing,
                position: pos,
                marks,
                active,
                with_marks: true,
                empty: Some("Elige un clip en el panel de medios"),
                note: note.as_deref().map(|n| (n, theme::WARN)),
            },
        );
        if out.mark_in {
            self.set_in(mixer);
        }
        if out.mark_out {
            self.set_out(mixer);
        }
        if out.step_back {
            self.step_by(mixer, -1.0);
        }
        if out.step_fwd {
            self.step_by(mixer, 1.0);
        }
        if out.to_end {
            self.go_to_end(mixer);
        }
        if out.toggle {
            self.toggle(mixer);
        }
        if out.to_start {
            self.seek(mixer, 0.0);
        }
        if let Some(t) = out.scrub {
            let d = self.info().map_or(0.0, |i| i.duration);
            self.seek(mixer, t * d);
        }
        out.clicked
    }

    fn upload_frame(&mut self, ctx: &egui::Context) {
        let Some(p) = self.player.as_mut() else { return };
        let playing = p.is_playing();
        if let Some(frame) = p.frame()
            && self.shown_pts != Some(frame.pts)
        {
            monitor::upload(ctx, &mut self.texture, "visor-origen", frame);
            self.shown_pts = Some(frame.pts);
        }
        // Reproduciendo: repintar a cada vuelta. En pausa: lo justo para que llegue el
        // fotograma de un salto.
        if playing {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
    }
}
