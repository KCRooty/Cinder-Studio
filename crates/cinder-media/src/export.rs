//! Render final: convierte un [`RenderPlan`] (la línea de tiempo ya aplanada en tramos) en un
//! archivo H.264 o H.265 con audio AAC, fotograma a fotograma y sin pasar por el visor.
//!
//! - Imagen: cada fotograma de salida busca el fotograma del origen que le toca (decodificando
//!   hacia delante y saltando solo cuando hace falta), lo escala a la resolución de salida sin
//!   deformarlo (bandas negras si no encaja) y lo entrega al codificador. Los huecos salen en negro.
//! - Sonido: todos los tramos de audio se suman por bloques de un fotograma y van al
//!   codificador AAC en trozos del tamaño que pide.
//! - Codificador: se prueba primero el libre por software (x264/x265) y, si la compilación de
//!   FFmpeg no lo trae, los de hardware (NVENC, Quick Sync, AMF, Media Foundation…).

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use ff::format::Pixel;
use ff::software::{resampling, scaling};
use ff::util::channel_layout::ChannelLayout;
use ff::util::format::sample::{Sample, Type as SampleType};
use ff::{Dictionary, Rational, codec, encoder, frame};
use ffmpeg_next as ff;

use crate::MediaError;
use crate::decoder::resample_into;

/// Códec de vídeo de la entrega.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    H265,
}

impl VideoCodec {
    pub fn label(self) -> &'static str {
        match self {
            VideoCodec::H264 => "H.264",
            VideoCodec::H265 => "H.265 (HEVC)",
        }
    }

    /// Codificadores por orden de preferencia: primero el de software, luego los de hardware.
    fn candidates(self) -> &'static [&'static str] {
        match self {
            VideoCodec::H264 => {
                &["libx264", "h264_nvenc", "h264_qsv", "h264_amf", "h264_videotoolbox", "h264_mf", "libopenh264"]
            }
            VideoCodec::H265 => &["libx265", "hevc_nvenc", "hevc_qsv", "hevc_amf", "hevc_videotoolbox", "hevc_mf"],
        }
    }
}

/// Resolución de salida de un preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputSize {
    /// La del proyecto.
    Project,
    Fixed(u32, u32),
}

/// Cómo se codifica. La calidad es un CRF (menos es mejor) para x264/x265; los codificadores
/// de hardware no lo entienden y usan la tasa de bits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportSettings {
    pub codec: VideoCodec,
    pub crf: u8,
    /// Velocidad de x264/x265: `veryfast`, `medium`, `slow`…
    pub speed: &'static str,
    pub video_kbps: u32,
    pub audio_kbps: u32,
}

/// Un preset de entrega.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportPreset {
    pub name: &'static str,
    pub description: &'static str,
    pub size: OutputSize,
    /// Tope de fotogramas por segundo; por encima se baja a la mitad (60 → 30).
    pub max_fps: Option<u32>,
    pub settings: ExportSettings,
}

impl ExportPreset {
    pub fn all() -> Vec<ExportPreset> {
        vec![
            ExportPreset {
                name: "YouTube 1080p",
                description: "1920×1080, H.264 de alta calidad y AAC a 320 kb/s. Lo que YouTube recomienda subir.",
                size: OutputSize::Fixed(1920, 1080),
                max_fps: None,
                settings: ExportSettings {
                    codec: VideoCodec::H264,
                    crf: 18,
                    speed: "medium",
                    video_kbps: 16_000,
                    audio_kbps: 320,
                },
            },
            ExportPreset {
                name: "YouTube 4K",
                description: "3840×2160 en H.265, para que el archivo no pese el doble.",
                size: OutputSize::Fixed(3840, 2160),
                max_fps: None,
                settings: ExportSettings {
                    codec: VideoCodec::H265,
                    crf: 20,
                    speed: "medium",
                    video_kbps: 45_000,
                    audio_kbps: 320,
                },
            },
            ExportPreset {
                name: "TikTok / Shorts",
                description: "Vertical 1080×1920, H.264, máximo 60 fps. Lo horizontal queda con bandas.",
                size: OutputSize::Fixed(1080, 1920),
                max_fps: Some(60),
                settings: ExportSettings {
                    codec: VideoCodec::H264,
                    crf: 20,
                    speed: "medium",
                    video_kbps: 12_000,
                    audio_kbps: 192,
                },
            },
            ExportPreset {
                name: "Máster",
                description: "Resolución del proyecto en H.265 casi sin pérdida, para archivar o recodificar.",
                size: OutputSize::Project,
                max_fps: None,
                settings: ExportSettings {
                    codec: VideoCodec::H265,
                    crf: 12,
                    speed: "slow",
                    video_kbps: 80_000,
                    audio_kbps: 320,
                },
            },
        ]
    }

    /// Resolución y fotogramas por segundo de salida para un proyecto concreto.
    pub fn resolve(&self, project_size: (u32, u32), project_fps: (u32, u32)) -> ((u32, u32), (u32, u32)) {
        let (w, h) = match self.size {
            OutputSize::Project => project_size,
            OutputSize::Fixed(w, h) => (w, h),
        };
        let (num, den) = project_fps;
        let fps = match self.max_fps {
            Some(max) if u64::from(num) > u64::from(max) * u64::from(den.max(1)) => (num, den.saturating_mul(2)),
            _ => (num, den),
        };
        ((w.max(2) & !1, h.max(2) & !1), fps)
    }
}

/// Un trozo de un archivo colocado en la línea de tiempo de salida.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderSpan {
    pub path: PathBuf,
    /// Primer fotograma (de salida) en que aparece.
    pub start: i64,
    /// Segundo del archivo que corresponde a `start`.
    pub source_secs: f64,
    /// Duración en fotogramas de salida.
    pub frames: i64,
}

impl RenderSpan {
    fn end(&self) -> i64 {
        self.start + self.frames
    }
}

/// Todo lo que hace falta para renderizar, sin depender del modelo de proyecto.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderPlan {
    pub width: u32,
    pub height: u32,
    /// Fotogramas por segundo como fracción (30000/1001…).
    pub fps: (u32, u32),
    pub sample_rate: u32,
    /// Duración total en fotogramas.
    pub frames: i64,
    /// Ordenados y sin solaparse; lo que no cubren sale en negro.
    pub video: Vec<RenderSpan>,
    /// Se suman entre sí.
    pub audio: Vec<RenderSpan>,
}

impl RenderPlan {
    fn frame_secs(&self) -> f64 {
        f64::from(self.fps.1) / f64::from(self.fps.0.max(1))
    }

    /// Primera muestra de audio del fotograma `frame` (exacta también con 30000/1001).
    fn sample_at(&self, frame: i64) -> i64 {
        let v =
            i128::from(frame) * i128::from(self.sample_rate) * i128::from(self.fps.1) / i128::from(self.fps.0.max(1));
        v as i64
    }
}

/// Cómo fue el render.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportReport {
    /// Codificador de vídeo que se usó al final (`libx264`, `h264_nvenc`…).
    pub encoder: String,
    pub frames: i64,
    pub seconds: f64,
}

/// Renderiza `plan` en `output` (el contenedor sale de la extensión: `.mp4`, `.mov`, `.mkv`).
/// `progress(hecho, total)` se llama tras cada fotograma. Si `cancel` se activa, para, borra el
/// archivo a medias y devuelve [`MediaError::Cancelled`].
pub fn render(
    plan: &RenderPlan,
    settings: &ExportSettings,
    output: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(i64, i64),
) -> Result<ExportReport, MediaError> {
    crate::init()?;
    // x264, x265 y el AAC imprimen estadísticas por consola; solo interesan los errores.
    ff::log::set_level(ff::log::Level::Error);
    if plan.frames <= 0 {
        return Err(MediaError::Encode("la línea de tiempo está vacía".into()));
    }
    if plan.width < 2 || plan.height < 2 || plan.fps.0 == 0 || plan.fps.1 == 0 || plan.sample_rate == 0 {
        return Err(MediaError::Encode("ajustes de salida no válidos".into()));
    }
    let result = Renderer::new(plan, settings, output).and_then(|mut r| {
        let started = Instant::now();
        r.run(plan, cancel, &mut progress)?;
        Ok(ExportReport {
            encoder: r.encoder_name.clone(),
            frames: plan.frames,
            seconds: started.elapsed().as_secs_f64(),
        })
    });
    if result.is_err() {
        std::fs::remove_file(output).ok();
    }
    result
}

struct Renderer {
    octx: ff::format::context::Output,
    video: encoder::Video,
    video_index: usize,
    video_tb: Rational,
    /// Si el codificador no acepta YUV 4:2:0 plano, conversión final a su formato.
    to_encoder: Option<scaling::Context>,
    audio: encoder::Audio,
    audio_index: usize,
    audio_tb: Rational,
    audio_frame: usize,
    audio_fifo: VecDeque<f32>,
    audio_pts: i64,
    encoder_name: String,
}

impl Renderer {
    fn new(plan: &RenderPlan, settings: &ExportSettings, output: &Path) -> Result<Self, MediaError> {
        let mut octx = ff::format::output(output)
            .map_err(|e| MediaError::Encode(format!("no se pudo crear «{}»: {e}", output.display())))?;
        let global_header = octx.format().flags().contains(ff::format::Flags::GLOBAL_HEADER);
        let (video, encoder_name, pix) = open_video_encoder(plan, settings, global_header)?;
        let video_tb = Rational(plan.fps.1 as i32, plan.fps.0 as i32);
        let video_index = {
            let mut st = octx.add_stream(encoder::find_by_name(&encoder_name))?;
            st.set_time_base(video_tb);
            st.set_parameters(&video);
            st.index()
        };
        let to_encoder = if pix == Pixel::YUV420P {
            None
        } else {
            Some(scaling::Context::get(
                Pixel::YUV420P,
                plan.width,
                plan.height,
                pix,
                plan.width,
                plan.height,
                scaling::Flags::POINT,
            )?)
        };

        let audio = open_audio_encoder(plan.sample_rate, settings.audio_kbps, global_header)?;
        let audio_tb = Rational(1, plan.sample_rate as i32);
        let audio_index = {
            let mut st = octx.add_stream(encoder::find(codec::Id::AAC))?;
            st.set_time_base(audio_tb);
            st.set_parameters(&audio);
            st.index()
        };
        let audio_frame = match audio.frame_size() {
            0 => 1024,
            n => n as usize,
        };

        let mut opts = Dictionary::new();
        // El índice al principio: el vídeo empieza a verse antes de terminar de bajarse.
        opts.set("movflags", "+faststart");
        octx.write_header_with(opts)
            .map_err(|e| MediaError::Encode(format!("no se pudo escribir la cabecera: {e}")))?;
        Ok(Self {
            octx,
            video,
            video_index,
            video_tb,
            to_encoder,
            audio,
            audio_index,
            audio_tb,
            audio_frame,
            audio_fifo: VecDeque::new(),
            audio_pts: 0,
            encoder_name,
        })
    }

    fn run(
        &mut self,
        plan: &RenderPlan,
        cancel: &AtomicBool,
        progress: &mut impl FnMut(i64, i64),
    ) -> Result<(), MediaError> {
        let mut sources: HashMap<PathBuf, VideoSource> = HashMap::new();
        let mut mix = AudioMix::new(plan);
        let mut next_span = 0usize;
        let mut chunk: Vec<f32> = Vec::new();
        for f in 0..plan.frames {
            if cancel.load(Ordering::Relaxed) {
                return Err(MediaError::Cancelled);
            }
            // Imagen.
            while plan.video.get(next_span).is_some_and(|s| s.end() <= f) {
                next_span += 1;
            }
            let mut canvas = black_frame(plan.width, plan.height);
            if let Some(span) = plan.video.get(next_span).filter(|s| s.start <= f) {
                let secs = span.source_secs + (f - span.start) as f64 * plan.frame_secs();
                if !sources.contains_key(&span.path) {
                    sources.insert(span.path.clone(), VideoSource::open(&span.path)?);
                }
                if let Some(src) = sources.get_mut(&span.path) {
                    src.draw_at(secs, &mut canvas)?;
                }
            }
            canvas.set_pts(Some(f));
            self.send_video(&canvas)?;

            // Sonido del mismo fotograma.
            let (s0, s1) = (plan.sample_at(f), plan.sample_at(f + 1));
            chunk.clear();
            chunk.resize(((s1 - s0).max(0) as usize) * 2, 0.0);
            mix.render(plan, f, s0, &mut chunk)?;
            self.audio_fifo.extend(chunk.iter().map(|s| s.clamp(-1.0, 1.0)));
            self.drain_audio_fifo(false)?;

            progress(f + 1, plan.frames);
        }
        self.drain_audio_fifo(true)?;
        self.video.send_eof()?;
        self.write_video_packets()?;
        self.audio.send_eof()?;
        self.write_audio_packets()?;
        self.octx.write_trailer().map_err(|e| MediaError::Encode(format!("no se pudo cerrar el archivo: {e}")))?;
        Ok(())
    }

    fn send_video(&mut self, canvas: &frame::Video) -> Result<(), MediaError> {
        match self.to_encoder.as_mut() {
            Some(sws) => {
                let mut converted = frame::Video::empty();
                sws.run(canvas, &mut converted)?;
                converted.set_pts(canvas.pts());
                self.video.send_frame(&converted)?;
            }
            None => self.video.send_frame(canvas)?,
        }
        self.write_video_packets()
    }

    fn write_video_packets(&mut self) -> Result<(), MediaError> {
        let tb = self.octx.stream(self.video_index).map(|s| s.time_base()).unwrap_or(self.video_tb);
        let mut packet = ff::Packet::empty();
        while self.video.receive_packet(&mut packet).is_ok() {
            packet.set_stream(self.video_index);
            // Cada paquete dura un fotograma; sin esto el último queda sin duración y el archivo
            // dice durar un fotograma menos.
            if packet.duration() <= 0 {
                packet.set_duration(1);
            }
            packet.rescale_ts(self.video_tb, tb);
            packet
                .write_interleaved(&mut self.octx)
                .map_err(|e| MediaError::Encode(format!("escribiendo vídeo: {e}")))?;
        }
        Ok(())
    }

    fn drain_audio_fifo(&mut self, last: bool) -> Result<(), MediaError> {
        let n = self.audio_frame;
        while self.audio_fifo.len() >= n * 2 || (last && !self.audio_fifo.is_empty()) {
            let mut fr = frame::Audio::new(Sample::F32(SampleType::Planar), n, ChannelLayout::STEREO);
            fr.set_rate(self.audio.rate());
            for i in 0..n {
                let l = self.audio_fifo.pop_front().unwrap_or(0.0);
                let r = self.audio_fifo.pop_front().unwrap_or(0.0);
                if let Some(x) = fr.plane_mut::<f32>(0).get_mut(i) {
                    *x = l;
                }
                if let Some(x) = fr.plane_mut::<f32>(1).get_mut(i) {
                    *x = r;
                }
            }
            fr.set_pts(Some(self.audio_pts));
            self.audio_pts += n as i64;
            self.audio.send_frame(&fr)?;
            self.write_audio_packets()?;
        }
        Ok(())
    }

    fn write_audio_packets(&mut self) -> Result<(), MediaError> {
        let tb = self.octx.stream(self.audio_index).map(|s| s.time_base()).unwrap_or(self.audio_tb);
        let mut packet = ff::Packet::empty();
        while self.audio.receive_packet(&mut packet).is_ok() {
            packet.set_stream(self.audio_index);
            packet.rescale_ts(self.audio_tb, tb);
            packet
                .write_interleaved(&mut self.octx)
                .map_err(|e| MediaError::Encode(format!("escribiendo audio: {e}")))?;
        }
        Ok(())
    }
}

fn open_video_encoder(
    plan: &RenderPlan,
    settings: &ExportSettings,
    global_header: bool,
) -> Result<(encoder::Video, String, Pixel), MediaError> {
    let mut tried = Vec::new();
    for &name in settings.codec.candidates() {
        let Some(codec) = encoder::find_by_name(name) else { continue };
        let pix = match codec.video().ok().and_then(|v| v.formats().map(|f| f.collect::<Vec<_>>())) {
            Some(list) if !list.contains(&Pixel::YUV420P) && list.contains(&Pixel::NV12) => Pixel::NV12,
            Some(list) if !list.contains(&Pixel::YUV420P) => {
                tried.push(format!("{name} (formato de píxel)"));
                continue;
            }
            _ => Pixel::YUV420P,
        };
        let mut enc = codec::context::Context::new_with_codec(codec).encoder().video()?;
        enc.set_width(plan.width);
        enc.set_height(plan.height);
        enc.set_format(pix);
        enc.set_time_base(Rational(plan.fps.1 as i32, plan.fps.0 as i32));
        enc.set_frame_rate(Some(Rational(plan.fps.0 as i32, plan.fps.1 as i32)));
        enc.set_gop(((f64::from(plan.fps.0) / f64::from(plan.fps.1)) * 2.0).round().max(1.0) as u32);
        enc.set_max_b_frames(2);
        enc.set_colorspace(ff::color::Space::BT709);
        enc.set_color_range(ff::color::Range::MPEG);
        enc.set_bit_rate(settings.video_kbps as usize * 1000);
        if global_header {
            enc.set_flags(codec::Flags::GLOBAL_HEADER);
        }
        let mut opts = Dictionary::new();
        match name {
            "libx264" | "libx265" => {
                // Con CRF manda la calidad; la tasa de bits se ignora.
                enc.set_bit_rate(0);
                opts.set("crf", &settings.crf.to_string());
                opts.set("preset", settings.speed);
                if name == "libx265" {
                    opts.set("x265-params", "log-level=error");
                    opts.set("tag", "hvc1");
                }
            }
            "h264_nvenc" | "hevc_nvenc" => {
                opts.set("preset", "p5");
                opts.set("rc", "vbr");
            }
            _ => {}
        }
        match enc.open_with(opts) {
            Ok(opened) => return Ok((opened, name.to_string(), pix)),
            Err(e) => tried.push(format!("{name} ({e})")),
        }
    }
    Err(MediaError::NoEncoder(if tried.is_empty() {
        format!("esta compilación de FFmpeg no trae ningún codificador {}", settings.codec.label())
    } else {
        format!("no se pudo abrir ningún codificador {}: {}", settings.codec.label(), tried.join(", "))
    }))
}

fn open_audio_encoder(rate: u32, kbps: u32, global_header: bool) -> Result<encoder::Audio, MediaError> {
    let codec = encoder::find(codec::Id::AAC).ok_or_else(|| MediaError::Encode("no hay codificador AAC".into()))?;
    let mut enc = codec::context::Context::new_with_codec(codec).encoder().audio()?;
    enc.set_rate(rate as i32);
    enc.set_format(Sample::F32(SampleType::Planar));
    enc.set_channel_layout(ChannelLayout::STEREO);
    enc.set_bit_rate(kbps as usize * 1000);
    enc.set_time_base(Rational(1, rate as i32));
    if global_header {
        enc.set_flags(codec::Flags::GLOBAL_HEADER);
    }
    Ok(enc.open_as(codec)?)
}

/// Fotograma YUV 4:2:0 negro (rango limitado: Y=16, U=V=128).
fn black_frame(w: u32, h: u32) -> frame::Video {
    let mut f = frame::Video::new(Pixel::YUV420P, w, h);
    f.data_mut(0).fill(16);
    f.data_mut(1).fill(128);
    f.data_mut(2).fill(128);
    f
}

/// Rectángulo (par) donde cabe una imagen `sw×sh` dentro de `dw×dh` sin deformarse.
fn fit(sw: u32, sh: u32, dw: u32, dh: u32) -> (u32, u32, u32, u32) {
    let scale = (f64::from(dw) / f64::from(sw)).min(f64::from(dh) / f64::from(sh));
    let w = ((f64::from(sw) * scale).round() as u32).clamp(2, dw) & !1;
    let h = ((f64::from(sh) * scale).round() as u32).clamp(2, dh) & !1;
    (((dw - w) / 2) & !1, ((dh - h) / 2) & !1, w, h)
}

/// Lector de fotogramas de un archivo para el render: avanza decodificando y solo salta cuando
/// le piden algo de atrás o muy por delante.
struct VideoSource {
    input: ff::format::context::Input,
    index: usize,
    decoder: ff::decoder::Video,
    time_base: f64,
    /// Media duración de un fotograma del origen: margen para decidir cuál toca.
    half_frame: f64,
    current: Option<(f64, frame::Video)>,
    pending: Option<(f64, frame::Video)>,
    eof: bool,
    scaler: Option<(scaling::Context, (u32, u32, u32, u32))>,
    scaled: frame::Video,
}

impl VideoSource {
    fn open(path: &Path) -> Result<Self, MediaError> {
        let input = ff::format::input(path)
            .map_err(|e| MediaError::Open { path: path.display().to_string(), reason: e.to_string() })?;
        let (index, time_base, decoder, fps) = {
            let stream = input.streams().best(ff::media::Type::Video).ok_or(MediaError::NoVideo)?;
            let ctx = ff::codec::Context::from_parameters(stream.parameters())?;
            let fps = f64::from(stream.avg_frame_rate());
            (stream.index(), f64::from(stream.time_base()), ctx.decoder().video()?, fps)
        };
        let fps = if fps.is_finite() && fps > 0.0 { fps } else { 30.0 };
        Ok(Self {
            input,
            index,
            decoder,
            time_base,
            half_frame: 0.5 / fps,
            current: None,
            pending: None,
            eof: false,
            scaler: None,
            scaled: frame::Video::empty(),
        })
    }

    fn seek(&mut self, secs: f64) {
        let ts = (secs.max(0.0) * f64::from(ff::ffi::AV_TIME_BASE)) as i64;
        if self.input.seek(ts, ..ts).is_err() {
            self.input.seek(0, ..0).ok();
        }
        self.decoder.flush();
        self.current = None;
        self.pending = None;
        self.eof = false;
    }

    /// El siguiente fotograma decodificado, o `None` al acabar el archivo.
    fn decode_next(&mut self) -> Option<(f64, frame::Video)> {
        loop {
            let mut decoded = frame::Video::empty();
            if self.decoder.receive_frame(&mut decoded).is_ok() {
                let pts = decoded.timestamp().unwrap_or(0) as f64 * self.time_base;
                return Some((pts, decoded));
            }
            if self.eof {
                return None;
            }
            let mut sent = false;
            for (stream, packet) in self.input.packets() {
                if stream.index() == self.index {
                    self.decoder.send_packet(&packet).ok();
                    sent = true;
                    break;
                }
            }
            if !sent {
                self.decoder.send_eof().ok();
                self.eof = true;
            }
        }
    }

    /// Pinta en `canvas` el fotograma del origen que corresponde al segundo `secs`.
    fn draw_at(&mut self, secs: f64, canvas: &mut frame::Video) -> Result<(), MediaError> {
        let behind = self.current.as_ref().is_some_and(|(pts, _)| secs + self.half_frame < *pts);
        let far = self.current.as_ref().is_some_and(|(pts, _)| secs > *pts + 3.0);
        if behind || far || (self.current.is_none() && self.pending.is_none() && secs > 0.5) {
            self.seek(secs);
        }
        loop {
            if self.pending.is_none() {
                self.pending = self.decode_next();
            }
            match &self.pending {
                Some((pts, _)) if *pts < secs + self.half_frame - 1e-6 || self.current.is_none() => {
                    let first = self.current.is_none();
                    self.current = self.pending.take();
                    if first && self.current.as_ref().is_some_and(|(pts, _)| *pts > secs + self.half_frame) {
                        break;
                    }
                }
                _ => break,
            }
        }
        let Some((_, src)) = self.current.as_ref() else { return Ok(()) };
        let (sw, sh) = (src.width(), src.height());
        if sw == 0 || sh == 0 {
            return Ok(());
        }
        let rect = fit(sw, sh, canvas.width(), canvas.height());
        let key = (sw, sh, rect.2, rect.3);
        let format = src.format();
        let rebuild = self.scaler.as_ref().is_none_or(|(s, k)| *k != key || s.input().format != format);
        if rebuild {
            let ctx = scaling::Context::get(format, sw, sh, Pixel::YUV420P, rect.2, rect.3, scaling::Flags::BICUBIC)?;
            self.scaler = Some((ctx, key));
        }
        if let Some((sws, _)) = self.scaler.as_mut() {
            sws.run(src, &mut self.scaled)?;
        }
        blit(&self.scaled, canvas, rect.0, rect.1);
        Ok(())
    }
}

/// Copia un fotograma YUV 4:2:0 dentro de otro en la posición (x, y), que tiene que ser par.
fn blit(src: &frame::Video, dst: &mut frame::Video, x: u32, y: u32) {
    for plane in 0..3 {
        let shift = u32::from(plane > 0);
        let (w, h) = ((src.width() >> shift) as usize, (src.height() >> shift) as usize);
        let (px, py) = ((x >> shift) as usize, (y >> shift) as usize);
        let (ss, ds) = (src.stride(plane), dst.stride(plane));
        let src_data = src.data(plane);
        let dst_data = dst.data_mut(plane);
        for row in 0..h {
            let (Some(from), Some(to)) = (
                src_data.get(row * ss..row * ss + w),
                dst_data.get_mut((py + row) * ds + px..(py + row) * ds + px + w),
            ) else {
                break;
            };
            to.copy_from_slice(from);
        }
    }
}

/// Mezcla de los tramos de audio, bloque a bloque.
struct AudioMix {
    readers: HashMap<usize, AudioReader>,
    next: usize,
}

impl AudioMix {
    fn new(_plan: &RenderPlan) -> Self {
        Self { readers: HashMap::new(), next: 0 }
    }

    /// Suma en `out` (estéreo intercalado, desde la muestra `s0`) lo que suena en el fotograma `f`.
    fn render(&mut self, plan: &RenderPlan, f: i64, s0: i64, out: &mut [f32]) -> Result<(), MediaError> {
        // Abrir los tramos que empiezan en este fotograma.
        while let Some(span) = plan.audio.get(self.next).filter(|s| s.start <= f) {
            if span.end() > f {
                // Si el tramo ya había empezado (no pasa al renderizar desde 0), entra por donde va.
                let offset = (f - span.start) as f64 * plan.frame_secs();
                match AudioReader::open(&span.path, span.source_secs + offset, plan.sample_rate) {
                    Ok(r) => {
                        self.readers.insert(self.next, r);
                    }
                    // Un medio sin audio no para el render: ese tramo queda en silencio.
                    Err(MediaError::NoAudio) => {}
                    Err(e) => return Err(e),
                }
            }
            self.next += 1;
        }
        let s1 = s0 + (out.len() / 2) as i64;
        let mut finished = Vec::new();
        for (&i, reader) in self.readers.iter_mut() {
            let Some(span) = plan.audio.get(i) else { continue };
            let (ss, se) = (plan.sample_at(span.start), plan.sample_at(span.end()));
            let (a, b) = (ss.max(s0), se.min(s1));
            if a < b {
                let range = ((a - s0) as usize * 2)..((b - s0) as usize * 2);
                if let Some(dst) = out.get_mut(range) {
                    reader.mix_into(dst);
                }
            }
            if se <= s1 {
                finished.push(i);
            }
        }
        for i in finished {
            self.readers.remove(&i);
        }
        Ok(())
    }
}

/// Lector de audio que se va pidiendo por trozos (f32 estéreo intercalado a `rate`).
struct AudioReader {
    input: ff::format::context::Input,
    index: usize,
    decoder: ff::decoder::Audio,
    resampler: Option<resampling::Context>,
    time_base: f64,
    rate: u32,
    start: f64,
    buffer: VecDeque<f32>,
    scratch: Vec<f32>,
    eof: bool,
}

impl AudioReader {
    fn open(path: &Path, start: f64, rate: u32) -> Result<Self, MediaError> {
        let mut input = ff::format::input(path)
            .map_err(|e| MediaError::Open { path: path.display().to_string(), reason: e.to_string() })?;
        let (index, time_base, decoder) = {
            let stream = input.streams().best(ff::media::Type::Audio).ok_or(MediaError::NoAudio)?;
            let ctx = ff::codec::Context::from_parameters(stream.parameters())?;
            (stream.index(), f64::from(stream.time_base()), ctx.decoder().audio()?)
        };
        let start = start.max(0.0);
        if start > 0.0 {
            let ts = (start * f64::from(ff::ffi::AV_TIME_BASE)) as i64;
            if input.seek(ts, ..ts).is_err() {
                input.seek(0, ..0).ok();
            }
        }
        Ok(Self {
            input,
            index,
            decoder,
            resampler: None,
            time_base,
            rate,
            start,
            buffer: VecDeque::new(),
            scratch: Vec::new(),
            eof: false,
        })
    }

    /// Suma las siguientes muestras en `out`. Si el archivo se acaba, lo que falta es silencio.
    fn mix_into(&mut self, out: &mut [f32]) {
        while self.buffer.len() < out.len() && !self.eof {
            self.fill();
        }
        for s in out.iter_mut() {
            match self.buffer.pop_front() {
                Some(v) => *s += v,
                None => break,
            }
        }
    }

    fn fill(&mut self) {
        let mut decoded = frame::Audio::empty();
        if self.decoder.receive_frame(&mut decoded).is_ok() {
            let pts = decoded.timestamp().unwrap_or(0) as f64 * self.time_base;
            let dur = decoded.samples() as f64 / f64::from(decoded.rate().max(1));
            if pts + dur <= self.start {
                return;
            }
            self.scratch.clear();
            if resample_into(&mut self.resampler, self.rate, &decoded, &mut self.scratch).is_err() {
                return;
            }
            let skip = if pts < self.start {
                (((self.start - pts) * f64::from(self.rate)) as usize * 2).min(self.scratch.len())
            } else {
                0
            };
            self.start = 0.0;
            self.buffer.extend(self.scratch.get(skip..).unwrap_or(&[]));
            return;
        }
        for (stream, packet) in self.input.packets() {
            if stream.index() == self.index {
                self.decoder.send_packet(&packet).ok();
                return;
            }
        }
        if self.decoder.send_eof().is_err() {
            self.eof = true;
            return;
        }
        // Lo que quedara dentro del decodificador.
        while self.decoder.receive_frame(&mut decoded).is_ok() {
            self.scratch.clear();
            if resample_into(&mut self.resampler, self.rate, &decoded, &mut self.scratch).is_ok() {
                self.buffer.extend(self.scratch.iter());
            }
        }
        self.eof = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encaja_sin_deformar_y_en_pares() {
        // 16:9 en vertical 9:16: bandas arriba y abajo.
        assert_eq!(fit(1920, 1080, 1080, 1920), (0, 656, 1080, 608));
        // Igual proporción: ocupa todo.
        assert_eq!(fit(1280, 720, 1920, 1080), (0, 0, 1920, 1080));
        // 4:3 en 16:9: bandas a los lados.
        assert_eq!(fit(640, 480, 1920, 1080), (240, 0, 1440, 1080));
    }

    #[test]
    fn las_muestras_por_fotograma_cuadran_en_ntsc() {
        let plan = RenderPlan {
            width: 2,
            height: 2,
            fps: (30000, 1001),
            sample_rate: 48_000,
            frames: 0,
            video: vec![],
            audio: vec![],
        };
        // 1001 fotogramas a 29,97 son exactamente 33,4 s = 1 603 200 muestras.
        assert_eq!(plan.sample_at(30_000), 48_000 * 1001);
        assert_eq!(plan.sample_at(1) - plan.sample_at(0), 1601);
    }

    #[test]
    fn tiktok_baja_los_120_fps_a_60() {
        let tiktok = ExportPreset::all().into_iter().find(|p| p.name.starts_with("TikTok")).unwrap();
        assert_eq!(tiktok.resolve((1920, 1080), (120, 1)).1, (120, 2));
        assert_eq!(tiktok.resolve((1920, 1080), (30, 1)), ((1080, 1920), (30, 1)));
        let master = ExportPreset::all().into_iter().find(|p| p.name == "Máster").unwrap();
        assert_eq!(master.resolve((1281, 721), (25, 1)).0, (1280, 720));
    }
}
