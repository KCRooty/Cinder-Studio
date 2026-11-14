//! Reproductor de un clip: abre el archivo, arranca el hilo de decodificación y la salida de
//! audio, y lleva el reloj. La interfaz solo llama a [`Player::frame`] en cada repintado.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use ffmpeg_next as ff;

use crate::MediaError;
use crate::audio::{self, AudioOut, AudioShared};
use crate::decoder::{AudioSink, Command, DecoderThread};

/// Un fotograma listo para el visor: RGBA de 8 bits sin premultiplicar.
#[derive(Clone)]
pub struct VideoFrame {
    /// Instante del fotograma, en segundos desde el principio del clip.
    pub pts: f64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub(crate) generation: u64,
}

impl std::fmt::Debug for VideoFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoFrame")
            .field("pts", &self.pts)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

/// Lo que se sabe del clip al abrirlo.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaInfo {
    pub path: PathBuf,
    /// Duración en segundos (0 si el contenedor no la dice).
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    /// Fotogramas por segundo del vídeo (0 si no se sabe).
    pub fps: f64,
    pub video_codec: String,
    pub audio_codec: Option<String>,
    pub audio_rate: Option<u32>,
}

impl MediaInfo {
    /// Lee el contenedor sin decodificar nada.
    pub fn probe(path: &Path) -> Result<Self, MediaError> {
        let input = open_input(path)?;
        info_of(&input, path)
    }
}

/// Reloj de reproducción: con audio, cuenta las muestras que han sonado; sin audio, el
/// reloj de pared.
enum Clock {
    Audio { shared: Arc<AudioShared>, rate: u32 },
    Wall { since: Option<Instant> },
}

pub struct Player {
    info: MediaInfo,
    commands: Sender<Command>,
    frames: Receiver<VideoFrame>,
    thread: Option<JoinHandle<()>>,
    _audio: Option<AudioOut>,
    audio_error: Option<String>,
    clock: Clock,
    /// Segundo desde el que cuenta el reloj (el del último salto).
    base: f64,
    /// Lo que llevaba contado el reloj de pared antes de la última pausa.
    paused_at: f64,
    playing: bool,
    generation: u64,
    current: Option<VideoFrame>,
    next: Option<VideoFrame>,
}

impl Player {
    /// Abre un clip. Si no hay tarjeta de sonido, reproduce igual sin audio y lo dice en
    /// [`Player::audio_error`].
    pub fn open(path: &Path) -> Result<Self, MediaError> {
        Self::open_with(path, true)
    }

    /// Como [`Player::open`], pero sin intentar abrir la tarjeta de sonido (tests, render).
    pub fn open_silent(path: &Path) -> Result<Self, MediaError> {
        Self::open_with(path, false)
    }

    fn open_with(path: &Path, with_audio: bool) -> Result<Self, MediaError> {
        crate::init()?;
        let input = open_input(path)?;
        let info = info_of(&input, path)?;
        let video_index = input.streams().best(ff::media::Type::Video).map(|s| s.index()).ok_or(MediaError::NoVideo)?;
        let audio_index = input.streams().best(ff::media::Type::Audio).map(|s| s.index());

        let (frames_tx, frames_rx) = crossbeam_channel::bounded(12);
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();

        let mut audio_error = None;
        let mut audio_out = None;
        let mut clock = Clock::Wall { since: None };
        let mut sink = None;
        if let (Some(_), true) = (audio_index, with_audio) {
            match audio::default_rate() {
                Ok(rate) => {
                    // Un segundo de estéreo de margen entre el decodificador y la tarjeta.
                    let (producer, consumer) = rtrb::RingBuffer::new(rate as usize * 2);
                    let shared = AudioShared::new();
                    match audio::open(consumer, Arc::clone(&shared)) {
                        Ok(out) => {
                            clock = Clock::Audio { shared: Arc::clone(&shared), rate: out.rate };
                            sink = Some(AudioSink { producer, rate: out.rate, shared });
                            audio_out = Some(out);
                        }
                        Err(e) => audio_error = Some(e.to_string()),
                    }
                }
                Err(e) => audio_error = Some(e.to_string()),
            }
        }

        // Los contextos de escalado y remuestreo de FFmpeg no se pueden mover entre hilos:
        // el decodificador se construye ya dentro de su hilo y avisa de si arrancó.
        let (ready_tx, ready_rx) = crossbeam_channel::bounded::<Result<(), MediaError>>(1);
        let thread = std::thread::Builder::new()
            .name("cinder-decodificacion".into())
            .spawn(move || match DecoderThread::new(input, video_index, audio_index, sink, frames_tx, cmd_rx) {
                Ok(decoder) => {
                    ready_tx.send(Ok(())).ok();
                    decoder.run();
                }
                Err(e) => {
                    ready_tx.send(Err(e.into())).ok();
                }
            })
            .map_err(|e| MediaError::Thread(e.to_string()))?;
        ready_rx.recv().map_err(|e| MediaError::Thread(e.to_string()))??;

        Ok(Self {
            info,
            commands: cmd_tx,
            frames: frames_rx,
            thread: Some(thread),
            _audio: audio_out,
            audio_error,
            clock,
            base: 0.0,
            paused_at: 0.0,
            playing: false,
            generation: 0,
            current: None,
            next: None,
        })
    }

    pub fn info(&self) -> &MediaInfo {
        &self.info
    }

    /// Por qué no suena, si no suena.
    pub fn audio_error(&self) -> Option<&str> {
        self.audio_error.as_deref()
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Segundo actual de la reproducción.
    pub fn position(&self) -> f64 {
        let t = match &self.clock {
            Clock::Audio { shared, rate } => {
                self.base + shared.played.load(Ordering::Acquire) as f64 / f64::from((*rate).max(1))
            }
            Clock::Wall { since } => {
                self.base + self.paused_at + since.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0)
            }
        };
        if self.info.duration > 0.0 { t.min(self.info.duration) } else { t }
    }

    pub fn play(&mut self) {
        if self.playing {
            return;
        }
        if self.info.duration > 0.0 && self.position() >= self.info.duration - 1e-3 {
            self.seek(0.0);
        }
        self.playing = true;
        match &mut self.clock {
            Clock::Audio { shared, .. } => shared.playing.store(true, Ordering::Release),
            Clock::Wall { since } => *since = Some(Instant::now()),
        }
    }

    pub fn pause(&mut self) {
        if !self.playing {
            return;
        }
        self.playing = false;
        match &mut self.clock {
            Clock::Audio { shared, .. } => shared.playing.store(false, Ordering::Release),
            Clock::Wall { since } => {
                if let Some(s) = since.take() {
                    self.paused_at += s.elapsed().as_secs_f64();
                }
            }
        }
    }

    pub fn toggle(&mut self) {
        if self.playing { self.pause() } else { self.play() }
    }

    /// Salta a un segundo del clip. Responde al momento; el fotograma llega en cuanto se decodifica.
    pub fn seek(&mut self, secs: f64) {
        let max = if self.info.duration > 0.0 { self.info.duration } else { f64::MAX };
        let secs = secs.clamp(0.0, max);
        self.generation += 1;
        self.base = secs;
        self.paused_at = 0.0;
        match &mut self.clock {
            Clock::Audio { shared, .. } => shared.played.store(0, Ordering::Release),
            Clock::Wall { since } => {
                if since.is_some() {
                    *since = Some(Instant::now());
                }
            }
        }
        self.next = None;
        // Se queda el fotograma viejo en pantalla hasta que llega el nuevo, sin parpadeo.
        self.commands.send(Command::Seek { secs, generation: self.generation }).ok();
    }

    /// El fotograma que toca enseñar ahora. Llamar en cada repintado.
    pub fn frame(&mut self) -> Option<&VideoFrame> {
        let now = self.position();
        let mut fresh_current = self.current.as_ref().is_some_and(|c| c.generation == self.generation);
        loop {
            let candidate = match self.next.take() {
                Some(f) => f,
                None => match self.frames.try_recv() {
                    Ok(f) => f,
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
                },
            };
            if candidate.generation != self.generation {
                continue; // de antes del último salto
            }
            // Tras un salto se enseña el primer fotograma que llegue, aunque vaya un poco adelantado.
            if candidate.pts <= now + 1e-3 || !fresh_current {
                self.current = Some(candidate);
                fresh_current = true;
            } else {
                self.next = Some(candidate);
                break;
            }
        }
        if self.playing && self.info.duration > 0.0 && now >= self.info.duration - 1e-3 {
            self.pause();
        }
        self.current.as_ref()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.commands.send(Command::Stop).ok();
        // Vaciar el canal desbloquea al decodificador si estaba esperando sitio.
        while self.frames.try_recv().is_ok() {}
        if let Some(t) = self.thread.take() {
            t.join().ok();
        }
    }
}

fn open_input(path: &Path) -> Result<ff::format::context::Input, MediaError> {
    crate::init()?;
    ff::format::input(path).map_err(|e| MediaError::Open { path: path.display().to_string(), reason: e.to_string() })
}

fn info_of(input: &ff::format::context::Input, path: &Path) -> Result<MediaInfo, MediaError> {
    let v = input.streams().best(ff::media::Type::Video).ok_or(MediaError::NoVideo)?;
    let vctx = ff::codec::Context::from_parameters(v.parameters())?;
    let vdec = vctx.decoder().video()?;
    let rate = v.avg_frame_rate();
    let fps = if rate.denominator() != 0 { f64::from(rate) } else { 0.0 };
    let duration = if input.duration() > 0 { input.duration() as f64 / f64::from(ff::ffi::AV_TIME_BASE) } else { 0.0 };
    let (audio_codec, audio_rate) = match input.streams().best(ff::media::Type::Audio) {
        Some(a) => {
            let actx = ff::codec::Context::from_parameters(a.parameters())?;
            let adec = actx.decoder().audio()?;
            (Some(adec.id().name().to_string()), Some(adec.rate()))
        }
        None => (None, None),
    };
    Ok(MediaInfo {
        path: path.to_path_buf(),
        duration,
        width: vdec.width(),
        height: vdec.height(),
        fps,
        video_codec: vdec.id().name().to_string(),
        audio_codec,
        audio_rate,
    })
}
