//! Decodificación de solo audio: para las voces del mezclador y para las formas de onda.

use std::path::Path;

use ff::software::resampling;
use ffmpeg_next as ff;

use crate::MediaError;
use crate::decoder::resample_into;

/// Decodifica el audio de `path` desde `start_secs`, en f32 estéreo intercalado a `rate`, y se
/// lo va pasando a `sink` por trozos. `sink` devuelve `false` para parar.
pub(crate) fn decode_audio(
    path: &Path,
    start_secs: f64,
    rate: u32,
    mut sink: impl FnMut(&[f32]) -> bool,
) -> Result<(), MediaError> {
    crate::init()?;
    let mut input = ff::format::input(path)
        .map_err(|e| MediaError::Open { path: path.display().to_string(), reason: e.to_string() })?;
    let (index, time_base, mut decoder) = {
        let stream = input.streams().best(ff::media::Type::Audio).ok_or(MediaError::NoAudio)?;
        let ctx = ff::codec::Context::from_parameters(stream.parameters())?;
        (stream.index(), f64::from(stream.time_base()), ctx.decoder().audio()?)
    };
    let start = start_secs.max(0.0);
    if start > 0.0 {
        let ts = (start * f64::from(ff::ffi::AV_TIME_BASE)) as i64;
        if input.seek(ts, ..ts).is_err() {
            input.seek(0, ..0).ok();
        }
    }
    let mut state = Drain { resampler: None, out: Vec::new(), time_base, rate, start };
    for (stream, packet) in input.packets() {
        if stream.index() != index {
            continue;
        }
        decoder.send_packet(&packet).ok();
        if !state.run(&mut decoder, &mut sink) {
            return Ok(());
        }
    }
    decoder.send_eof().ok();
    state.run(&mut decoder, &mut sink);
    Ok(())
}

struct Drain {
    resampler: Option<resampling::Context>,
    out: Vec<f32>,
    time_base: f64,
    rate: u32,
    start: f64,
}

impl Drain {
    /// Saca lo que tenga el decodificador. Devuelve `false` si `sink` pidió parar.
    fn run(&mut self, decoder: &mut ff::decoder::Audio, sink: &mut impl FnMut(&[f32]) -> bool) -> bool {
        let mut decoded = ff::frame::Audio::empty();
        while decoder.receive_frame(&mut decoded).is_ok() {
            let pts = decoded.timestamp().unwrap_or(0) as f64 * self.time_base;
            let dur = decoded.samples() as f64 / f64::from(decoded.rate().max(1));
            if pts + dur <= self.start {
                continue;
            }
            self.out.clear();
            if resample_into(&mut self.resampler, self.rate, &decoded, &mut self.out).is_err() {
                continue;
            }
            // Recorte fino: fuera las muestras anteriores al punto pedido.
            let skip = if pts < self.start {
                (((self.start - pts) * f64::from(self.rate)) as usize * 2).min(self.out.len())
            } else {
                0
            };
            if skip < self.out.len() && !sink(&self.out[skip..]) {
                return false;
            }
        }
        true
    }
}

/// Picos de la forma de onda: el valor absoluto máximo de cada tramo, `per_second` tramos por
/// segundo. Lee el archivo entero: llamar desde un hilo aparte.
pub fn peaks(path: &Path, per_second: u32) -> Result<Vec<f32>, MediaError> {
    const RATE: u32 = 8000;
    let bucket = (RATE / per_second.max(1)).max(1) as usize * 2;
    let mut peaks = Vec::new();
    let (mut max, mut filled) = (0.0f32, 0usize);
    decode_audio(path, 0.0, RATE, |samples| {
        for &s in samples {
            max = max.max(s.abs());
            filled += 1;
            if filled == bucket {
                peaks.push(max.min(1.0));
                max = 0.0;
                filled = 0;
            }
        }
        true
    })?;
    if filled > 0 {
        peaks.push(max.min(1.0));
    }
    Ok(peaks)
}
