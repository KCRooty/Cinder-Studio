//! Hilo de decodificación: lee paquetes con FFmpeg, decodifica vídeo (a RGBA) y audio
//! (a f32 estéreo intercalado a la frecuencia de la tarjeta) y los entrega.
//!
//! El vídeo va por un canal acotado y el audio por una cola sin bloqueo, así que el
//! decodificador nunca se adelanta más de lo que caben en ellos. Mientras espera sitio sigue
//! atendiendo órdenes, para que un salto responda al momento.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError};
use ff::format::Pixel;
use ff::software::{resampling, scaling};
use ff::util::channel_layout::ChannelLayout;
use ff::util::format::sample::{Sample, Type as SampleType};
use ffmpeg_next as ff;
use rtrb::Producer;

use crate::audio::AudioShared;
use crate::player::VideoFrame;

/// Ancho máximo de los fotogramas que llegan al visor. El render final no pasa por aquí.
pub(crate) const MAX_PREVIEW_WIDTH: u32 = 1920;

pub(crate) enum Command {
    /// Saltar a este segundo; la generación marca los fotogramas que vengan después.
    Seek {
        secs: f64,
        generation: u64,
    },
    Stop,
}

pub(crate) struct AudioSink {
    pub producer: Producer<f32>,
    pub rate: u32,
    pub shared: Arc<AudioShared>,
}

struct VideoPart {
    index: usize,
    decoder: ff::decoder::Video,
    scaler: Option<scaling::Context>,
    time_base: f64,
}

struct AudioPart {
    index: usize,
    decoder: ff::decoder::Audio,
    resampler: Option<resampling::Context>,
    time_base: f64,
    sink: AudioSink,
}

enum Flow {
    Continue,
    Seek(f64, u64),
    Stop,
}

pub(crate) struct DecoderThread {
    input: ff::format::context::Input,
    video: VideoPart,
    audio: Option<AudioPart>,
    frames: Sender<VideoFrame>,
    commands: Receiver<Command>,
    generation: u64,
    /// Tras un salto se descartan los fotogramas anteriores a este punto.
    skip_until: f64,
}

impl DecoderThread {
    pub fn new(
        input: ff::format::context::Input,
        video_index: usize,
        audio_index: Option<usize>,
        audio_sink: Option<AudioSink>,
        frames: Sender<VideoFrame>,
        commands: Receiver<Command>,
    ) -> Result<Self, ff::Error> {
        let vstream = input.stream(video_index).ok_or(ff::Error::StreamNotFound)?;
        let vtb = f64::from(vstream.time_base());
        let vctx = ff::codec::Context::from_parameters(vstream.parameters())?;
        let video = VideoPart { index: video_index, decoder: vctx.decoder().video()?, scaler: None, time_base: vtb };

        let audio = match (audio_index, audio_sink) {
            (Some(index), Some(sink)) => {
                let astream = input.stream(index).ok_or(ff::Error::StreamNotFound)?;
                let atb = f64::from(astream.time_base());
                let actx = ff::codec::Context::from_parameters(astream.parameters())?;
                Some(AudioPart { index, decoder: actx.decoder().audio()?, resampler: None, time_base: atb, sink })
            }
            _ => None,
        };
        Ok(Self { input, video, audio, frames, commands, generation: 0, skip_until: 0.0 })
    }

    pub fn run(mut self) {
        loop {
            match self.poll_command() {
                Flow::Stop => return,
                Flow::Seek(secs, generation) => self.seek(secs, generation),
                Flow::Continue => {}
            }
            let next = self.input.packets().next();
            match next {
                Some((stream, packet)) => {
                    let index = stream.index();
                    let flow = if index == self.video.index {
                        self.video.decoder.send_packet(&packet).ok();
                        self.drain_video()
                    } else if self.audio.as_ref().is_some_and(|a| a.index == index) {
                        if let Some(a) = self.audio.as_mut() {
                            a.decoder.send_packet(&packet).ok();
                        }
                        self.drain_audio()
                    } else {
                        Flow::Continue
                    };
                    match flow {
                        Flow::Stop => return,
                        Flow::Seek(secs, generation) => self.seek(secs, generation),
                        Flow::Continue => {}
                    }
                }
                None => {
                    // Fin del archivo: vaciar lo que quede en los decodificadores y esperar órdenes.
                    self.video.decoder.send_eof().ok();
                    let mut flow = self.drain_video();
                    if let (Flow::Continue, Some(a)) = (&flow, self.audio.as_mut()) {
                        a.decoder.send_eof().ok();
                        flow = self.drain_audio();
                    }
                    let flow = match flow {
                        Flow::Continue => match self.commands.recv() {
                            Ok(Command::Seek { secs, generation }) => Flow::Seek(secs, generation),
                            Ok(Command::Stop) | Err(_) => Flow::Stop,
                        },
                        other => other,
                    };
                    match flow {
                        Flow::Stop => return,
                        Flow::Seek(secs, generation) => self.seek(secs, generation),
                        Flow::Continue => {}
                    }
                }
            }
        }
    }

    fn poll_command(&self) -> Flow {
        poll(&self.commands)
    }

    fn seek(&mut self, secs: f64, generation: u64) {
        let ts = (secs.max(0.0) * f64::from(ff::ffi::AV_TIME_BASE)) as i64;
        // Al fotograma clave anterior; lo que sobra hasta `secs` se descarta al decodificar.
        if self.input.seek(ts, ..ts).is_err() {
            self.input.seek(0, ..0).ok();
        }
        self.video.decoder.flush();
        if let Some(a) = self.audio.as_mut() {
            a.decoder.flush();
            a.resampler = None;
            a.sink.shared.flush.store(true, Ordering::Release);
            // El callback vacía la cola en su siguiente vuelta; se espera a que lo haga para
            // no meter audio nuevo que luego tiraría. Si no hay callback (sin tarjeta), no espera.
            for _ in 0..200 {
                if !a.sink.shared.flush.load(Ordering::Acquire) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            a.sink.shared.flush.store(false, Ordering::Release);
        }
        self.generation = generation;
        self.skip_until = secs;
    }

    fn drain_video(&mut self) -> Flow {
        let mut decoded = ff::frame::Video::empty();
        while self.video.decoder.receive_frame(&mut decoded).is_ok() {
            let pts = decoded.timestamp().unwrap_or(0) as f64 * self.video.time_base;
            let frame_dur = self.video.decoder.frame_rate().map(|r| 1.0 / f64::from(r).max(1.0)).unwrap_or(1.0 / 30.0);
            if pts + frame_dur * 0.5 < self.skip_until {
                continue;
            }
            let Some(frame) = self.convert_frame(&decoded, pts) else { continue };
            let mut pending = frame;
            loop {
                match self.frames.try_send(pending) {
                    Ok(()) => break,
                    Err(TrySendError::Disconnected(_)) => return Flow::Stop,
                    Err(TrySendError::Full(back)) => {
                        pending = back;
                        match self.poll_command() {
                            Flow::Continue => std::thread::sleep(Duration::from_millis(4)),
                            other => return other,
                        }
                    }
                }
            }
        }
        Flow::Continue
    }

    fn convert_frame(&mut self, decoded: &ff::frame::Video, pts: f64) -> Option<VideoFrame> {
        let (sw, sh) = (decoded.width(), decoded.height());
        if sw == 0 || sh == 0 {
            return None;
        }
        let (dw, dh) = if sw > MAX_PREVIEW_WIDTH {
            let h = (u64::from(sh) * u64::from(MAX_PREVIEW_WIDTH) / u64::from(sw)) as u32;
            (MAX_PREVIEW_WIDTH, h.max(2) & !1)
        } else {
            (sw, sh)
        };
        let rebuild =
            self.video.scaler.as_ref().is_none_or(|s| {
                s.input().width != sw || s.input().height != sh || s.input().format != decoded.format()
            });
        if rebuild {
            self.video.scaler =
                scaling::Context::get(decoded.format(), sw, sh, Pixel::RGBA, dw, dh, scaling::Flags::BILINEAR).ok();
        }
        let scaler = self.video.scaler.as_mut()?;
        let mut rgba = ff::frame::Video::empty();
        scaler.run(decoded, &mut rgba).ok()?;
        let stride = rgba.stride(0);
        let row = dw as usize * 4;
        let data = rgba.data(0);
        let mut pixels = Vec::with_capacity(row * dh as usize);
        for y in 0..dh as usize {
            let start = y * stride;
            pixels.extend_from_slice(data.get(start..start + row)?);
        }
        Some(VideoFrame { pts, width: dw, height: dh, rgba: pixels, generation: self.generation })
    }

    fn drain_audio(&mut self) -> Flow {
        let skip_until = self.skip_until;
        let Some(a) = self.audio.as_mut() else { return Flow::Continue };
        let mut decoded = ff::frame::Audio::empty();
        let mut out: Vec<f32> = Vec::new();
        while a.decoder.receive_frame(&mut decoded).is_ok() {
            let pts = decoded.timestamp().unwrap_or(0) as f64 * a.time_base;
            let dur = decoded.samples() as f64 / f64::from(decoded.rate().max(1));
            if pts + dur <= skip_until {
                continue;
            }
            out.clear();
            if resample_into(&mut a.resampler, a.sink.rate, &decoded, &mut out).is_err() {
                continue;
            }
            // Recorte fino: quitar las muestras anteriores al punto de salto.
            let mut start = 0usize;
            if pts < skip_until {
                let frames = ((skip_until - pts) * f64::from(a.sink.rate)) as usize;
                start = (frames * 2).min(out.len());
            }
            let mut i = start;
            while i < out.len() {
                let free = a.sink.producer.slots();
                if free == 0 {
                    let flow = poll(&self.commands);
                    match flow {
                        Flow::Continue => std::thread::sleep(Duration::from_millis(4)),
                        other => return other,
                    }
                    continue;
                }
                let end = (i + free).min(out.len());
                for &s in &out[i..end] {
                    // Hay sitio comprobado; si aun así falla, la muestra se pierde.
                    a.sink.producer.push(s).ok();
                }
                i = end;
            }
        }
        Flow::Continue
    }
}

/// Lee las órdenes pendientes. Si llegan varios saltos seguidos (arrastrando la barra),
/// solo cuenta el último.
fn poll(commands: &Receiver<Command>) -> Flow {
    let mut flow = Flow::Continue;
    loop {
        match commands.try_recv() {
            Ok(Command::Seek { secs, generation }) => flow = Flow::Seek(secs, generation),
            Ok(Command::Stop) | Err(TryRecvError::Disconnected) => return Flow::Stop,
            Err(TryRecvError::Empty) => return flow,
        }
    }
}

/// Convierte un fotograma de audio a f32 estéreo intercalado a la frecuencia `rate`.
pub(crate) fn resample_into(
    resampler: &mut Option<resampling::Context>,
    rate: u32,
    input: &ff::frame::Audio,
    out: &mut Vec<f32>,
) -> Result<(), ff::Error> {
    let target = Sample::F32(SampleType::Packed);
    let layout = if input.channel_layout().channels() > 0 {
        input.channel_layout()
    } else {
        ChannelLayout::default(i32::from(input.channels().max(1)))
    };
    if resampler.is_none() {
        *resampler =
            Some(resampling::Context::get(input.format(), layout, input.rate(), target, ChannelLayout::STEREO, rate)?);
    }
    let Some(resampler) = resampler.as_mut() else { return Ok(()) };
    let mut converted = ff::frame::Audio::empty();
    let mut delay = resampler.run(input, &mut converted)?;
    push_packed(&converted, out);
    while delay.is_some() {
        let mut more = ff::frame::Audio::new(target, 1024, ChannelLayout::STEREO);
        delay = resampler.flush(&mut more)?;
        if more.samples() == 0 {
            break;
        }
        push_packed(&more, out);
    }
    Ok(())
}

fn push_packed(frame: &ff::frame::Audio, out: &mut Vec<f32>) {
    let n = frame.samples() * 2 * 4;
    let bytes = frame.data(0);
    let bytes = bytes.get(..n.min(bytes.len())).unwrap_or(&[]);
    out.extend(bytes.chunks_exact(4).map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]])));
}
