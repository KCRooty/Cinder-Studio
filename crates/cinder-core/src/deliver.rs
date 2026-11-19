//! Entrega: qué suena y qué se ve en cada fotograma de la línea de tiempo, en tramos listos
//! para renderizar.
//!
//! La imagen es la de la pista de vídeo visible más alta, igual que en el visor de programa:
//! [`video_spans`] la aplana en tramos que no se solapan. El sonido es la suma de todos los
//! clips de las pistas de audio sin silenciar ([`audio_spans`]).

use crate::timeline::{Frames, MediaId, Timeline, TrackKind};

/// Un trozo de un medio colocado en la línea de tiempo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub media: MediaId,
    /// Primer fotograma en la línea de tiempo.
    pub start: Frames,
    /// Primer fotograma usado del medio de origen.
    pub source_in: Frames,
    pub duration: Frames,
}

impl Span {
    pub fn end(&self) -> Frames {
        self.start + self.duration
    }
}

/// La imagen de la línea de tiempo en tramos ordenados y sin solaparse. Los huecos (negro) no
/// salen. Dos tramos seguidos del mismo medio y continuos en el origen se juntan en uno.
pub fn video_spans(timeline: &Timeline) -> Vec<Span> {
    let visible: Vec<_> = timeline.tracks.iter().filter(|t| t.kind == TrackKind::Video && !t.muted).collect();
    let mut cuts: Vec<Frames> =
        visible.iter().flat_map(|t| t.clips().iter().flat_map(|c| [c.start, c.end()])).collect();
    cuts.sort_unstable();
    cuts.dedup();

    let mut spans: Vec<Span> = Vec::new();
    for pair in cuts.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let Some(clip) = visible.iter().rev().find_map(|t| t.clip_at(a)) else { continue };
        let span = Span { media: clip.media, start: a, source_in: clip.source_in + (a - clip.start), duration: b - a };
        match spans.last_mut() {
            Some(last)
                if last.media == span.media
                    && last.end() == span.start
                    && last.source_in + last.duration == span.source_in =>
            {
                last.duration += span.duration;
            }
            _ => spans.push(span),
        }
    }
    spans
}

/// Todos los clips de las pistas de audio sin silenciar, ordenados por su inicio.
pub fn audio_spans(timeline: &Timeline) -> Vec<Span> {
    let mut spans: Vec<Span> = timeline
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio && !t.muted)
        .flat_map(|t| t.clips())
        .map(|c| Span { media: c.media, start: c.start, source_in: c.source_in, duration: c.duration })
        .collect();
    spans.sort_by_key(|s| s.start);
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeline::{Clip, ClipId, Track};

    fn clip(id: u64, media: u64, start: Frames, source_in: Frames, duration: Frames) -> Clip {
        Clip { id: ClipId(id), media: MediaId(media), start, source_in, duration, link: None }
    }

    fn span(media: u64, start: Frames, source_in: Frames, duration: Frames) -> Span {
        Span { media: MediaId(media), start, source_in, duration }
    }

    #[test]
    fn la_pista_mas_alta_tapa_a_las_de_abajo() {
        let mut v1 = Track::new(TrackKind::Video, "V1");
        v1.insert(clip(1, 1, 0, 0, 100)).unwrap();
        let mut v2 = Track::new(TrackKind::Video, "V2");
        v2.insert(clip(2, 2, 30, 500, 20)).unwrap();
        let tl = Timeline { tracks: vec![v1, v2] };
        assert_eq!(video_spans(&tl), [span(1, 0, 0, 30), span(2, 30, 500, 20), span(1, 50, 50, 50)]);
    }

    #[test]
    fn una_pista_oculta_no_sale_y_los_huecos_tampoco() {
        let mut v1 = Track::new(TrackKind::Video, "V1");
        v1.insert(clip(1, 1, 10, 0, 10)).unwrap();
        v1.insert(clip(2, 1, 40, 0, 10)).unwrap();
        let mut v2 = Track::new(TrackKind::Video, "V2");
        v2.muted = true;
        v2.insert(clip(3, 2, 0, 0, 100)).unwrap();
        let tl = Timeline { tracks: vec![v1, v2] };
        assert_eq!(video_spans(&tl), [span(1, 10, 0, 10), span(1, 40, 0, 10)]);
    }

    #[test]
    fn dos_clips_continuos_del_mismo_medio_se_juntan() {
        // Un clip cortado con la cuchilla sigue siendo una sola lectura del archivo.
        let mut v1 = Track::new(TrackKind::Video, "V1");
        v1.insert(clip(1, 1, 0, 100, 25)).unwrap();
        v1.insert(clip(2, 1, 25, 125, 25)).unwrap();
        let tl = Timeline { tracks: vec![v1] };
        assert_eq!(video_spans(&tl), [span(1, 0, 100, 50)]);
    }

    #[test]
    fn el_audio_suma_todas_las_pistas_sin_silenciar() {
        let mut a1 = Track::new(TrackKind::Audio, "A1");
        a1.insert(clip(1, 1, 20, 0, 10)).unwrap();
        let mut a2 = Track::new(TrackKind::Audio, "A2");
        a2.insert(clip(2, 2, 0, 5, 50)).unwrap();
        let mut a3 = Track::new(TrackKind::Audio, "A3");
        a3.muted = true;
        a3.insert(clip(3, 3, 0, 0, 50)).unwrap();
        let tl = Timeline { tracks: vec![a1, a2, a3] };
        assert_eq!(audio_spans(&tl), [span(2, 0, 5, 50), span(1, 20, 0, 10)]);
    }
}
