use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Posiciones y duraciones en fotogramas de la línea de tiempo.
pub type Frames = i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MediaId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClipId(pub u64);

/// Fracción exacta, para fotogramas por segundo como 30000/1001.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rational {
    pub num: u32,
    pub den: u32,
}

impl Rational {
    pub const fn new(num: u32, den: u32) -> Self {
        Self { num, den }
    }

    pub fn as_f64(self) -> f64 {
        f64::from(self.num) / f64::from(self.den)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clip {
    pub id: ClipId,
    pub media: MediaId,
    /// Primer fotograma del clip en la línea de tiempo.
    pub start: Frames,
    /// Primer fotograma usado del medio de origen.
    pub source_in: Frames,
    pub duration: Frames,
}

impl Clip {
    pub fn end(&self) -> Frames {
        self.start + self.duration
    }

    fn overlaps(&self, start: Frames, end: Frames) -> bool {
        self.start < end && start < self.end()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackKind {
    Video,
    Audio,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub kind: TrackKind,
    pub name: String,
    /// Siempre ordenados por `start` y sin solaparse.
    clips: Vec<Clip>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EditError {
    #[error("no existe la pista {0}")]
    NoSuchTrack(usize),
    #[error("no existe el clip {0:?}")]
    NoSuchClip(ClipId),
    #[error("la duración tiene que ser mayor que cero")]
    InvalidDuration,
    #[error("el clip empezaría antes del inicio de la línea de tiempo")]
    NegativeStart,
    #[error("el clip se solaparía con otro de la misma pista")]
    Overlap,
}

impl Track {
    pub fn new(kind: TrackKind, name: impl Into<String>) -> Self {
        Self { kind, name: name.into(), clips: Vec::new() }
    }

    pub fn clips(&self) -> &[Clip] {
        &self.clips
    }

    pub fn clip(&self, id: ClipId) -> Option<&Clip> {
        self.clips.iter().find(|c| c.id == id)
    }

    pub fn end(&self) -> Frames {
        self.clips.last().map_or(0, Clip::end)
    }

    fn check_free(&self, start: Frames, duration: Frames, ignore: Option<ClipId>) -> Result<(), EditError> {
        if duration <= 0 {
            return Err(EditError::InvalidDuration);
        }
        if start < 0 {
            return Err(EditError::NegativeStart);
        }
        let end = start + duration;
        let clash = self.clips.iter().any(|c| Some(c.id) != ignore && c.overlaps(start, end));
        if clash { Err(EditError::Overlap) } else { Ok(()) }
    }

    pub fn insert(&mut self, clip: Clip) -> Result<(), EditError> {
        self.check_free(clip.start, clip.duration, None)?;
        let at = self.clips.partition_point(|c| c.start < clip.start);
        self.clips.insert(at, clip);
        Ok(())
    }

    pub fn remove(&mut self, id: ClipId) -> Result<Clip, EditError> {
        let at = self.clips.iter().position(|c| c.id == id).ok_or(EditError::NoSuchClip(id))?;
        Ok(self.clips.remove(at))
    }

    /// Mueve un clip y devuelve dónde estaba.
    pub fn move_clip(&mut self, id: ClipId, new_start: Frames) -> Result<Frames, EditError> {
        let clip = self.clip(id).ok_or(EditError::NoSuchClip(id))?;
        let (old_start, duration) = (clip.start, clip.duration);
        self.check_free(new_start, duration, Some(id))?;
        let mut clip = self.remove(id)?;
        clip.start = new_start;
        self.insert(clip)?;
        Ok(old_start)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Timeline {
    pub tracks: Vec<Track>,
}

impl Timeline {
    /// Una pista de vídeo y una de audio, como un proyecto nuevo en blanco.
    pub fn with_default_tracks() -> Self {
        Self { tracks: vec![Track::new(TrackKind::Video, "V1"), Track::new(TrackKind::Audio, "A1")] }
    }

    pub fn track(&self, index: usize) -> Result<&Track, EditError> {
        self.tracks.get(index).ok_or(EditError::NoSuchTrack(index))
    }

    pub fn track_mut(&mut self, index: usize) -> Result<&mut Track, EditError> {
        self.tracks.get_mut(index).ok_or(EditError::NoSuchTrack(index))
    }

    pub fn duration(&self) -> Frames {
        self.tracks.iter().map(Track::end).max().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(id: u64, start: Frames, duration: Frames) -> Clip {
        Clip { id: ClipId(id), media: MediaId(1), start, source_in: 0, duration }
    }

    #[test]
    fn insert_keeps_clips_sorted() {
        let mut t = Track::new(TrackKind::Video, "V1");
        t.insert(clip(1, 100, 50)).unwrap();
        t.insert(clip(2, 0, 50)).unwrap();
        t.insert(clip(3, 50, 50)).unwrap();
        let starts: Vec<_> = t.clips().iter().map(|c| c.start).collect();
        assert_eq!(starts, [0, 50, 100]);
        assert_eq!(t.end(), 150);
    }

    #[test]
    fn insert_rejects_overlap_and_bad_ranges() {
        let mut t = Track::new(TrackKind::Video, "V1");
        t.insert(clip(1, 10, 20)).unwrap();
        assert_eq!(t.insert(clip(2, 25, 10)), Err(EditError::Overlap));
        assert_eq!(t.insert(clip(3, 0, 0)), Err(EditError::InvalidDuration));
        assert_eq!(t.insert(clip(4, -5, 3)), Err(EditError::NegativeStart));
        assert!(t.insert(clip(5, 30, 5)).is_ok(), "pegado al final no se solapa");
    }

    #[test]
    fn move_clip_can_slide_over_its_own_old_position() {
        let mut t = Track::new(TrackKind::Video, "V1");
        t.insert(clip(1, 0, 100)).unwrap();
        assert_eq!(t.move_clip(ClipId(1), 40), Ok(0));
        assert_eq!(t.clip(ClipId(1)).map(|c| c.start), Some(40));
    }

    #[test]
    fn move_clip_rejects_overlap_without_changing_anything() {
        let mut t = Track::new(TrackKind::Video, "V1");
        t.insert(clip(1, 0, 10)).unwrap();
        t.insert(clip(2, 20, 10)).unwrap();
        let before = t.clone();
        assert_eq!(t.move_clip(ClipId(1), 15), Err(EditError::Overlap));
        assert_eq!(t, before);
    }

    #[test]
    fn rational_ntsc() {
        assert!((Rational::new(30000, 1001).as_f64() - 29.97).abs() < 0.001);
    }
}
