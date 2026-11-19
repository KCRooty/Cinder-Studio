//! Render final con el clip de prueba (2 s, 320×180 a 25 fps, H.264 + AAC).

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use cinder_media::{ExportPreset, ExportSettings, MediaInfo, RenderPlan, RenderSpan, VideoCodec, render};

fn clip() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/prueba-2s.mp4")
}

fn out(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    dir.join(name)
}

/// Las compilaciones LGPL de FFmpeg (las de Windows del CI) no traen x264 ni x265, y en una
/// máquina sin GPU tampoco abre ningún codificador de hardware: ahí la prueba no tiene con qué.
fn sin_codificador(r: &Result<cinder_media::ExportReport, cinder_media::MediaError>) -> bool {
    if let Err(cinder_media::MediaError::NoEncoder(msg)) = r {
        eprintln!("se salta: {msg}");
        return true;
    }
    false
}

fn settings(codec: VideoCodec) -> ExportSettings {
    ExportSettings { codec, crf: 23, speed: "ultrafast", video_kbps: 2_000, audio_kbps: 128 }
}

/// 1 s del clip desde el segundo 0,5, un hueco de medio segundo y luego el clip desde el
/// principio; el audio, el clip entero desde el fotograma 0.
fn plan(width: u32, height: u32) -> RenderPlan {
    RenderPlan {
        width,
        height,
        fps: (25, 1),
        sample_rate: 48_000,
        frames: 75,
        video: vec![
            RenderSpan { path: clip(), start: 0, source_secs: 0.5, frames: 25 },
            RenderSpan { path: clip(), start: 37, source_secs: 0.0, frames: 38 },
        ],
        audio: vec![RenderSpan { path: clip(), start: 0, source_secs: 0.0, frames: 50 }],
    }
}

#[test]
fn renderiza_en_h264_con_huecos_y_audio() -> Result<(), Box<dyn std::error::Error>> {
    let path = out("entrega-h264.mp4");
    let mut last = (0, 0);
    let r = render(&plan(640, 360), &settings(VideoCodec::H264), &path, &AtomicBool::new(false), |d, t| {
        last = (d, t);
    });
    if sin_codificador(&r) {
        return Ok(());
    }
    let report = r?;
    assert_eq!(last, (75, 75));
    assert_eq!(report.frames, 75);
    let info = MediaInfo::probe(&path)?;
    assert_eq!((info.width, info.height), (640, 360));
    assert_eq!(info.video_codec, "h264");
    assert_eq!(info.audio_codec.as_deref(), Some("aac"));
    assert!((info.fps - 25.0).abs() < 0.01, "fps {}", info.fps);
    assert!((info.duration - 3.0).abs() < 0.1, "duración {}", info.duration);
    Ok(())
}

#[test]
fn renderiza_en_vertical_con_h265() -> Result<(), Box<dyn std::error::Error>> {
    let path = out("entrega-h265.mp4");
    let r = render(&plan(360, 640), &settings(VideoCodec::H265), &path, &AtomicBool::new(false), |_, _| {});
    if sin_codificador(&r) {
        return Ok(());
    }
    r?;
    let info = MediaInfo::probe(&path)?;
    assert_eq!((info.width, info.height), (360, 640));
    assert_eq!(info.video_codec, "hevc");
    Ok(())
}

#[test]
fn cancelar_para_y_no_deja_archivo() {
    let path = out("entrega-cancelada.mp4");
    let cancel = AtomicBool::new(true);
    let r = render(&plan(320, 180), &settings(VideoCodec::H264), &path, &cancel, |_, _| {});
    if sin_codificador(&r) {
        return;
    }
    assert!(matches!(r, Err(cinder_media::MediaError::Cancelled)), "{r:?}");
    assert!(!path.exists());
}

#[test]
fn los_presets_dan_tamanos_pares() {
    for p in ExportPreset::all() {
        let ((w, h), _) = p.resolve((1921, 1081), (30, 1));
        assert!(w % 2 == 0 && h % 2 == 0, "{}", p.name);
    }
}
