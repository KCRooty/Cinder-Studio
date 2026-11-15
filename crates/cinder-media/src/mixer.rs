//! Mezclador de la línea de tiempo: una sola salida de audio en la que suenan a la vez los
//! clips de todas las pistas, cada uno colocado en su muestra exacta.
//!
//! El reloj del mezclador (muestras que han sonado mientras se reproduce) es el reloj maestro
//! del programa. Cada clip es una *voz*: un hilo decodifica su audio a una cola sin bloqueo y
//! el callback lo suma a la salida cuando el reloj llega a su posición. Una voz que llega
//! tarde tira lo atrasado y se engancha en su sitio; una programada para dentro de un rato
//! espera callada. Así los cortes suenan sin hueco y sin desfase.
//!
//! El callback no reserva memoria, no bloquea y no hace I/O (AGENTS.md §6): las voces le
//! llegan por una cola y las que ya no sirven vuelven por otra para soltarse fuera de él.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::MediaError;
use crate::audio_clip::decode_audio;

/// Voces que pueden sonar a la vez.
const MAX_VOICES: usize = 64;
/// Fotogramas de audio que se mezclan de una vez dentro del callback.
const BLOCK: usize = 2048;

#[derive(Debug, Default)]
struct MixShared {
    /// Fotogramas de audio que han sonado mientras se reproducía.
    clock: AtomicU64,
    playing: AtomicBool,
}

pub(crate) struct Voice {
    consumer: Consumer<f32>,
    /// A qué fotograma del reloj corresponde la siguiente muestra de la cola.
    position: u64,
    alive: Arc<AtomicBool>,
    gain: f32,
}

/// La parte que vive dentro del callback.
pub(crate) struct MixCore {
    voices: Vec<Voice>,
    incoming: Consumer<Voice>,
    dead: Producer<Voice>,
    shared: Arc<MixShared>,
}

impl MixCore {
    /// Recoge las voces nuevas y devuelve las que ya no sirven.
    fn admit(&mut self) {
        while let Ok(v) = self.incoming.pop() {
            if self.voices.len() < MAX_VOICES {
                self.voices.push(v);
            } else {
                self.dead.push(v).ok();
            }
        }
        let mut i = 0;
        while i < self.voices.len() {
            if !self.voices[i].alive.load(Ordering::Acquire) && !self.dead.is_full() {
                let v = self.voices.swap_remove(i);
                self.dead.push(v).ok();
            } else {
                i += 1;
            }
        }
    }

    /// Mezcla en `out` (estéreo intercalado) el siguiente tramo y avanza el reloj.
    pub(crate) fn render(&mut self, out: &mut [f32]) {
        self.admit();
        out.fill(0.0);
        if !self.shared.playing.load(Ordering::Acquire) {
            return;
        }
        let n = out.len() / 2;
        let now = self.shared.clock.load(Ordering::Acquire);
        for v in &mut self.voices {
            // Atrasada: tirar lo que ya debería haber sonado.
            if v.position < now {
                let behind = (now - v.position).min((v.consumer.slots() / 2) as u64) as usize;
                if let Ok(chunk) = v.consumer.read_chunk(behind * 2) {
                    chunk.commit_all();
                }
                v.position += behind as u64;
            }
            if v.position < now || v.position >= now + n as u64 {
                continue;
            }
            let offset = (v.position - now) as usize;
            let take = (n - offset).min(v.consumer.slots() / 2);
            let Ok(chunk) = v.consumer.read_chunk(take * 2) else { continue };
            let (a, b) = chunk.as_slices();
            for (dst, &src) in out[offset * 2..].iter_mut().zip(a.iter().chain(b)) {
                *dst += src * v.gain;
            }
            chunk.commit_all();
            v.position += take as u64;
        }
        self.shared.clock.store(now + n as u64, Ordering::Release);
    }
}

/// Mientras vive, el clip suena; al soltarlo se calla y su hilo termina.
pub struct AudioClipHandle {
    alive: Arc<AtomicBool>,
}

impl Drop for AudioClipHandle {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

/// Reloj de pared para cuando no hay tarjeta de sonido.
#[derive(Default)]
struct Wall {
    since: Option<Instant>,
    before: f64,
}

pub struct Mixer {
    _stream: Option<cpal::Stream>,
    shared: Arc<MixShared>,
    incoming: Producer<Voice>,
    dead: Consumer<Voice>,
    rate: u32,
    error: Option<String>,
    wall: Wall,
}

impl Default for Mixer {
    fn default() -> Self {
        Self::new()
    }
}

impl Mixer {
    /// Abre la salida por defecto. Si no hay, el mezclador sigue dando la hora (con el reloj
    /// del sistema) y lo dice en [`Mixer::error`].
    pub fn new() -> Self {
        let shared = Arc::new(MixShared::default());
        let (incoming_tx, incoming_rx) = RingBuffer::new(MAX_VOICES);
        let (dead_tx, dead_rx) = RingBuffer::new(MAX_VOICES * 2);
        let core = MixCore {
            voices: Vec::with_capacity(MAX_VOICES),
            incoming: incoming_rx,
            dead: dead_tx,
            shared: Arc::clone(&shared),
        };
        let (stream, rate, error) = match open(core) {
            Ok((stream, rate)) => (Some(stream), rate, None),
            Err(e) => (None, 48_000, Some(e.to_string())),
        };
        Self { _stream: stream, shared, incoming: incoming_tx, dead: dead_rx, rate, error, wall: Wall::default() }
    }

    /// Por qué no suena, si no suena.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn has_output(&self) -> bool {
        self._stream.is_some()
    }

    /// Fotogramas de audio por segundo del reloj.
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Fotogramas de audio transcurridos mientras se reproducía.
    pub fn clock(&self) -> u64 {
        if self.has_output() {
            self.shared.clock.load(Ordering::Acquire)
        } else {
            let secs = self.wall.before + self.wall.since.map_or(0.0, |s| s.elapsed().as_secs_f64());
            (secs * f64::from(self.rate)) as u64
        }
    }

    pub fn set_playing(&mut self, playing: bool) {
        self.shared.playing.store(playing, Ordering::Release);
        match (playing, self.wall.since) {
            (true, None) => self.wall.since = Some(Instant::now()),
            (false, Some(since)) => {
                self.wall.before += since.elapsed().as_secs_f64();
                self.wall.since = None;
            }
            _ => {}
        }
    }

    /// Programa un clip: `frames` fotogramas de audio de `path` desde `source_secs`, que
    /// empiezan a sonar cuando el reloj llegue a `at_clock`. Sin tarjeta devuelve `None`.
    pub fn add_clip(
        &mut self,
        path: &Path,
        source_secs: f64,
        frames: u64,
        at_clock: u64,
        gain: f32,
    ) -> Option<AudioClipHandle> {
        if !self.has_output() || frames == 0 {
            return None;
        }
        self.reap();
        // Medio segundo de estéreo de margen entre el decodificador y la tarjeta.
        let (producer, consumer) = RingBuffer::new(self.rate as usize);
        let alive = Arc::new(AtomicBool::new(true));
        let voice = Voice { consumer, position: at_clock, alive: Arc::clone(&alive), gain };
        self.incoming.push(voice).ok()?;
        let feeder =
            Feeder { path: path.to_path_buf(), source_secs, rate: self.rate, producer, alive: Arc::clone(&alive) };
        let spawned = std::thread::Builder::new().name("cinder-audio-clip".into()).spawn(move || feeder.run(frames));
        if spawned.is_err() {
            alive.store(false, Ordering::Release);
            return None;
        }
        Some(AudioClipHandle { alive })
    }

    /// Suelta las voces que el callback ya ha devuelto. Llamar de vez en cuando.
    pub fn reap(&mut self) {
        while self.dead.pop().is_ok() {}
    }
}

/// Hilo que decodifica el audio de un clip y lo mete en la cola de su voz.
struct Feeder {
    path: PathBuf,
    source_secs: f64,
    rate: u32,
    producer: Producer<f32>,
    alive: Arc<AtomicBool>,
}

impl Feeder {
    fn run(mut self, frames: u64) {
        let mut left = frames as usize * 2;
        let (producer, alive) = (&mut self.producer, &self.alive);
        decode_audio(&self.path, self.source_secs, self.rate, |samples| {
            let samples = &samples[..samples.len().min(left)];
            let mut i = 0;
            while i < samples.len() {
                if !alive.load(Ordering::Acquire) {
                    return false;
                }
                let free = producer.slots();
                if free == 0 {
                    std::thread::sleep(Duration::from_millis(3));
                    continue;
                }
                let end = (i + free).min(samples.len());
                for &s in &samples[i..end] {
                    producer.push(s).ok();
                }
                i = end;
            }
            left -= samples.len();
            left > 0
        })
        .ok();
    }
}

fn open(core: MixCore) -> Result<(cpal::Stream, u32), MediaError> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or_else(|| MediaError::Audio("no hay salida de audio".into()))?;
    let supported = device.default_output_config().map_err(|e| MediaError::Audio(e.to_string()))?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let rate = config.sample_rate;
    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, config, core)?,
        SampleFormat::I16 => build::<i16>(&device, config, core)?,
        SampleFormat::I32 => build::<i32>(&device, config, core)?,
        SampleFormat::U16 => build::<u16>(&device, config, core)?,
        other => return Err(MediaError::Audio(format!("formato de muestra no soportado: {other}"))),
    };
    stream.play().map_err(|e| MediaError::Audio(e.to_string()))?;
    Ok((stream, rate))
}

fn build<T>(device: &cpal::Device, config: cpal::StreamConfig, mut core: MixCore) -> Result<cpal::Stream, MediaError>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let channels = usize::from(config.channels).max(1);
    let silence = T::from_sample(0.0f32);
    let mut scratch = vec![0.0f32; BLOCK * 2];
    let callback = move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
        for block in data.chunks_mut(BLOCK * channels) {
            let n = block.len() / channels;
            let mix = &mut scratch[..n * 2];
            core.render(mix);
            for (frame, lr) in block.chunks_mut(channels).zip(mix.chunks(2)) {
                let (l, r) = (lr[0].clamp(-1.0, 1.0), lr[1].clamp(-1.0, 1.0));
                if channels == 1 {
                    frame[0] = T::from_sample((l + r) * 0.5);
                } else {
                    for (i, s) in frame.iter_mut().enumerate() {
                        *s = match i {
                            0 => T::from_sample(l),
                            1 => T::from_sample(r),
                            _ => silence,
                        };
                    }
                }
            }
        }
    };
    device
        .build_output_stream(config, callback, |e| eprintln!("cinder: error de audio: {e}"), None)
        .map_err(|e| MediaError::Audio(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un núcleo de mezcla sin tarjeta, con las colas a mano.
    fn core() -> (MixCore, Producer<Voice>, Consumer<Voice>, Arc<MixShared>) {
        let shared = Arc::new(MixShared::default());
        let (incoming_tx, incoming_rx) = RingBuffer::new(8);
        let (dead_tx, dead_rx) = RingBuffer::new(16);
        let core = MixCore {
            voices: Vec::with_capacity(MAX_VOICES),
            incoming: incoming_rx,
            dead: dead_tx,
            shared: Arc::clone(&shared),
        };
        (core, incoming_tx, dead_rx, shared)
    }

    /// Voz con `frames` fotogramas a un valor constante, colocada en `position`.
    fn voice(value: f32, frames: usize, position: u64) -> (Voice, Arc<AtomicBool>) {
        let (mut tx, rx) = RingBuffer::new(frames * 2);
        for _ in 0..frames * 2 {
            tx.push(value).unwrap();
        }
        let alive = Arc::new(AtomicBool::new(true));
        (Voice { consumer: rx, position, alive: Arc::clone(&alive), gain: 1.0 }, alive)
    }

    #[test]
    fn en_pausa_calla_y_el_reloj_no_avanza() {
        let (mut core, mut tx, _dead, shared) = core();
        tx.push(voice(0.5, 100, 0).0).unwrap();
        let mut out = [1.0f32; 64];
        core.render(&mut out);
        assert!(out.iter().all(|&s| s == 0.0));
        assert_eq!(shared.clock.load(Ordering::Acquire), 0);
    }

    #[test]
    fn cada_voz_entra_en_su_muestra_y_se_suman() {
        let (mut core, mut tx, _dead, shared) = core();
        shared.playing.store(true, Ordering::Release);
        tx.push(voice(0.25, 100, 0).0).unwrap();
        // Esta empieza en el fotograma 10 del reloj.
        tx.push(voice(0.5, 100, 10).0).unwrap();
        let mut out = [0.0f32; 40];
        core.render(&mut out);
        assert_eq!(out[0], 0.25);
        assert_eq!(out[19], 0.25, "fotograma 9: solo la primera");
        assert_eq!(out[20], 0.75, "fotograma 10: las dos");
        assert_eq!(shared.clock.load(Ordering::Acquire), 20);
    }

    #[test]
    fn una_voz_atrasada_tira_lo_viejo_y_se_engancha() {
        let (mut core, mut tx, _dead, shared) = core();
        shared.playing.store(true, Ordering::Release);
        shared.clock.store(1000, Ordering::Release);
        // Tenía que haber empezado en 990: sus 10 primeros fotogramas ya no suenan.
        let (mut ptx, prx) = RingBuffer::new(64);
        for i in 0..32 {
            ptx.push(i as f32).unwrap();
            ptx.push(i as f32).unwrap();
        }
        let alive = Arc::new(AtomicBool::new(true));
        tx.push(Voice { consumer: prx, position: 990, alive, gain: 1.0 }).unwrap();
        let mut out = [0.0f32; 8];
        core.render(&mut out);
        assert_eq!(out, [10.0, 10.0, 11.0, 11.0, 12.0, 12.0, 13.0, 13.0]);
    }

    #[test]
    fn una_voz_soltada_vuelve_para_liberarse_fuera_del_callback() {
        let (mut core, mut tx, mut dead, shared) = core();
        shared.playing.store(true, Ordering::Release);
        let (v, alive) = voice(0.5, 100, 0);
        tx.push(v).unwrap();
        let mut out = [0.0f32; 8];
        core.render(&mut out);
        assert_eq!(out[0], 0.5);
        alive.store(false, Ordering::Release);
        core.render(&mut out);
        assert!(out.iter().all(|&s| s == 0.0), "ya no suena");
        assert!(dead.pop().is_ok(), "y ha vuelto por la cola de salida");
    }
}
