//! Miniaturas para el panel de medios y la línea de tiempo.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::{MediaError, Player, VideoFrame};

/// Primer fotograma del clip, reducido a `max_width` píxeles de ancho como mucho.
pub fn thumbnail(path: &Path, max_width: u32) -> Result<VideoFrame, MediaError> {
    let mut player = Player::open_silent(path)?;
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(frame) = player.frame() {
            return Ok(shrink(frame, max_width.max(1)));
        }
        if Instant::now() > end {
            return Err(MediaError::Ffmpeg("no se pudo leer ningún fotograma".into()));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Reduce por un factor entero promediando cada bloque de píxeles.
fn shrink(frame: &VideoFrame, max_width: u32) -> VideoFrame {
    let factor = frame.width.div_ceil(max_width).max(1) as usize;
    if factor == 1 {
        return frame.clone();
    }
    let (sw, sh) = (frame.width as usize, frame.height as usize);
    let (dw, dh) = ((sw / factor).max(1), (sh / factor).max(1));
    let mut rgba = Vec::with_capacity(dw * dh * 4);
    for y in 0..dh {
        for x in 0..dw {
            let mut sum = [0u32; 4];
            let mut count = 0u32;
            for by in 0..factor {
                for bx in 0..factor {
                    let (px, py) = (x * factor + bx, y * factor + by);
                    if px < sw && py < sh {
                        let i = (py * sw + px) * 4;
                        if let Some(p) = frame.rgba.get(i..i + 4) {
                            for (s, &v) in sum.iter_mut().zip(p) {
                                *s += u32::from(v);
                            }
                            count += 1;
                        }
                    }
                }
            }
            let count = count.max(1);
            rgba.extend(sum.iter().map(|&s| (s / count) as u8));
        }
    }
    VideoFrame { pts: frame.pts, width: dw as u32, height: dh as u32, rgba, generation: 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduce_promediando_bloques() {
        // 4×2 píxeles: la mitad izquierda blanca, la derecha negra.
        let mut rgba = Vec::new();
        for _ in 0..2 {
            for x in 0..4 {
                let v = if x < 2 { 255 } else { 0 };
                rgba.extend([v, v, v, 255]);
            }
        }
        let small = shrink(&VideoFrame { pts: 0.0, width: 4, height: 2, rgba, generation: 0 }, 2);
        assert_eq!((small.width, small.height), (2, 1));
        assert_eq!(small.rgba, [255, 255, 255, 255, 0, 0, 0, 255]);
    }
}
