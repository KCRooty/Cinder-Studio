//! Salida de audio con cpal y estado compartido con el decodificador.
//!
//! El callback de audio no reserva memoria, no bloquea y no hace I/O (DESARROLLO.md §6): solo
//! saca muestras de una cola sin bloqueo (rtrb) y actualiza contadores atómicos.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use rtrb::Consumer;

use crate::MediaError;

/// Estado que comparten el callback de audio, el decodificador y la interfaz.
#[derive(Debug, Default)]
pub(crate) struct AudioShared {
    /// Fotogramas de audio (una muestra por canal) que han sonado desde el último salto.
    /// Cuenta también el silencio si la cola se queda vacía, para que el reloj no se pare
    /// y el vídeo no espere a un audio que viene detrás en el archivo.
    pub played: AtomicU64,
    /// Reproduciendo o en pausa.
    pub playing: AtomicBool,
    /// El decodificador pide vaciar la cola tras un salto; el callback la vacía y lo baja.
    pub flush: AtomicBool,
}

impl AudioShared {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

/// Stream de cpal abierto. Mientras vive, suena.
pub(crate) struct AudioOut {
    _stream: cpal::Stream,
    pub rate: u32,
}

/// Abre la salida por defecto. Devuelve la frecuencia a la que hay que remuestrear.
pub(crate) fn default_rate() -> Result<u32, MediaError> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or_else(|| MediaError::Audio("no hay salida de audio".into()))?;
    let config = device.default_output_config().map_err(|e| MediaError::Audio(e.to_string()))?;
    Ok(config.sample_rate())
}

pub(crate) fn open(consumer: Consumer<f32>, shared: Arc<AudioShared>) -> Result<AudioOut, MediaError> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or_else(|| MediaError::Audio("no hay salida de audio".into()))?;
    let supported = device.default_output_config().map_err(|e| MediaError::Audio(e.to_string()))?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let rate = config.sample_rate;
    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, config, consumer, shared)?,
        SampleFormat::I16 => build::<i16>(&device, config, consumer, shared)?,
        SampleFormat::I32 => build::<i32>(&device, config, consumer, shared)?,
        SampleFormat::U16 => build::<u16>(&device, config, consumer, shared)?,
        other => return Err(MediaError::Audio(format!("formato de muestra no soportado: {other}"))),
    };
    stream.play().map_err(|e| MediaError::Audio(e.to_string()))?;
    Ok(AudioOut { _stream: stream, rate })
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut consumer: Consumer<f32>,
    shared: Arc<AudioShared>,
) -> Result<cpal::Stream, MediaError>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let channels = usize::from(config.channels).max(1);
    let silence = T::from_sample(0.0f32);
    let callback = move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
        if shared.flush.load(Ordering::Acquire) {
            while consumer.pop().is_ok() {}
            shared.flush.store(false, Ordering::Release);
        }
        if !shared.playing.load(Ordering::Acquire) {
            data.fill(silence);
            return;
        }
        let mut frames = 0u64;
        for frame in data.chunks_mut(channels) {
            // La cola lleva estéreo intercalado (L, R).
            let l = consumer.pop().unwrap_or(0.0);
            let r = consumer.pop().unwrap_or(l);
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
            frames += 1;
        }
        shared.played.fetch_add(frames, Ordering::AcqRel);
    };
    device
        .build_output_stream(config, callback, |e| eprintln!("cinder: error de audio: {e}"), None)
        .map_err(|e| MediaError::Audio(e.to_string()))
}
