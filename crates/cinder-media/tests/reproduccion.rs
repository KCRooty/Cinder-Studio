//! Reproducción de un clip de prueba (2 s, 320×180 a 25 fps, H.264 + AAC a 48 kHz, generado
//! con las fuentes `testsrc2` y `sine` de FFmpeg). Sin tarjeta de sonido: reloj de pared.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use cinder_media::{MediaInfo, Player};

fn clip() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/prueba-2s.mp4")
}

/// Espera hasta que el visor tenga un fotograma que cumpla la condición.
fn wait_frame(p: &mut Player, ok: impl Fn(f64) -> bool) -> Option<f64> {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        if let Some(pts) = p.frame().map(|f| f.pts)
            && ok(pts)
        {
            return Some(pts);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    None
}

#[test]
fn lee_la_ficha_del_clip() -> Result<(), Box<dyn std::error::Error>> {
    let info = MediaInfo::probe(&clip())?;
    assert_eq!((info.width, info.height), (320, 180));
    assert!((info.fps - 25.0).abs() < 0.01, "fps {}", info.fps);
    assert!((info.duration - 2.0).abs() < 0.1, "duración {}", info.duration);
    assert_eq!(info.video_codec, "h264");
    assert_eq!(info.audio_codec.as_deref(), Some("aac"));
    assert_eq!(info.audio_rate, Some(48_000));
    Ok(())
}

#[test]
fn abre_y_ensena_el_primer_fotograma_en_pausa() -> Result<(), Box<dyn std::error::Error>> {
    let mut p = Player::open_silent(&clip())?;
    assert!(!p.is_playing());
    let pts = wait_frame(&mut p, |_| true).ok_or("no llegó ningún fotograma")?;
    assert!(pts.abs() < 0.001, "el primero debería ser el 0, es {pts}");
    let f = p.frame().ok_or("sin fotograma")?;
    assert_eq!((f.width, f.height), (320, 180));
    assert_eq!(f.rgba.len(), 320 * 180 * 4);
    // En pausa el reloj no avanza.
    std::thread::sleep(Duration::from_millis(100));
    assert!(p.position() < 0.001);
    Ok(())
}

#[test]
fn el_salto_cae_en_el_fotograma_pedido() -> Result<(), Box<dyn std::error::Error>> {
    let mut p = Player::open_silent(&clip())?;
    wait_frame(&mut p, |_| true).ok_or("no arrancó")?;
    // 1,24 s no es fotograma clave (hay uno por segundo): hay que decodificar desde el 1,0
    // y tirar lo que sobra.
    p.seek(1.24);
    let pts = wait_frame(&mut p, |t| t > 0.5).ok_or("el salto no llegó")?;
    assert!((pts - 1.24).abs() < 0.021, "fotograma {pts} en vez de 1,24");
    // Saltar atrás también.
    p.seek(0.4);
    let pts = wait_frame(&mut p, |t| t < 1.0).ok_or("el salto atrás no llegó")?;
    assert!((pts - 0.4).abs() < 0.021, "fotograma {pts} en vez de 0,4");
    Ok(())
}

#[test]
fn reproduce_avanza_y_se_para_al_final() -> Result<(), Box<dyn std::error::Error>> {
    let mut p = Player::open_silent(&clip())?;
    wait_frame(&mut p, |_| true).ok_or("no arrancó")?;
    p.seek(1.5);
    wait_frame(&mut p, |t| t >= 1.45).ok_or("el salto no llegó")?;
    p.play();
    let pts = wait_frame(&mut p, |t| t >= 1.9).ok_or("no llegó al final")?;
    assert!(pts <= 2.0);
    // Al acabar se pone en pausa sola y vuelve a empezar si se le da a reproducir.
    let end = Instant::now() + Duration::from_secs(2);
    while p.is_playing() && Instant::now() < end {
        p.frame();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!p.is_playing(), "debería pararse al final");
    p.play();
    let pts = wait_frame(&mut p, |t| t < 0.5).ok_or("no volvió al principio")?;
    assert!(pts < 0.5);
    Ok(())
}

#[test]
fn un_archivo_que_no_es_video_da_error_y_no_panico() {
    let bad = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    assert!(Player::open_silent(&bad).is_err());
    assert!(Player::open_silent(&PathBuf::from("/no/existe.mp4")).is_err());
}

/// Con tarjeta de sonido de verdad: el reloj lo lleva el audio. En CI no hay tarjeta,
/// así que se lanza a mano: `cargo test -p cinder-media -- --ignored`.
#[test]
#[ignore = "necesita salida de audio"]
fn con_tarjeta_el_reloj_va_con_el_audio() -> Result<(), Box<dyn std::error::Error>> {
    let mut p = Player::open(&clip())?;
    assert_eq!(p.audio_error(), None, "no abrió el audio");
    wait_frame(&mut p, |_| true).ok_or("no arrancó")?;
    let t0 = Instant::now();
    p.play();
    let mut last = 0.0;
    while t0.elapsed() < Duration::from_millis(1000) {
        if let Some(f) = p.frame() {
            last = f.pts;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let pos = p.position();
    assert!((pos - 1.0).abs() < 0.15, "tras 1 s el reloj marca {pos}");
    assert!((last - pos).abs() < 0.1, "vídeo {last} y audio {pos} desfasados");
    Ok(())
}
