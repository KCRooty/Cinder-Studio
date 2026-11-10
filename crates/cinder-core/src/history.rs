use serde::{Deserialize, Serialize};

use crate::timeline::{Clip, ClipId, EditError, Frames, Timeline};

/// Un cambio reversible en la línea de tiempo. Aplicarlo devuelve el cambio que lo deshace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Edit {
    AddClip { track: usize, clip: Clip },
    RemoveClip { track: usize, id: ClipId },
    MoveClip { track: usize, id: ClipId, to: Frames },
}

impl Edit {
    pub fn apply(self, timeline: &mut Timeline) -> Result<Edit, EditError> {
        match self {
            Edit::AddClip { track, clip } => {
                let id = clip.id;
                timeline.track_mut(track)?.insert(clip)?;
                Ok(Edit::RemoveClip { track, id })
            }
            Edit::RemoveClip { track, id } => {
                let clip = timeline.track_mut(track)?.remove(id)?;
                Ok(Edit::AddClip { track, clip })
            }
            Edit::MoveClip { track, id, to } => {
                let from = timeline.track_mut(track)?.move_clip(id, to)?;
                Ok(Edit::MoveClip { track, id, to: from })
            }
        }
    }
}

/// Historial de deshacer/rehacer, sin límite fijo.
#[derive(Debug, Default)]
pub struct History {
    undo: Vec<Edit>,
    redo: Vec<Edit>,
}

impl History {
    pub fn apply(&mut self, timeline: &mut Timeline, edit: Edit) -> Result<(), EditError> {
        let inverse = edit.apply(timeline)?;
        self.undo.push(inverse);
        self.redo.clear();
        Ok(())
    }

    /// Devuelve `false` si no había nada que deshacer.
    pub fn undo(&mut self, timeline: &mut Timeline) -> Result<bool, EditError> {
        let Some(edit) = self.undo.pop() else { return Ok(false) };
        let inverse = edit.apply(timeline)?;
        self.redo.push(inverse);
        Ok(true)
    }

    /// Devuelve `false` si no había nada que rehacer.
    pub fn redo(&mut self, timeline: &mut Timeline) -> Result<bool, EditError> {
        let Some(edit) = self.redo.pop() else { return Ok(false) };
        let inverse = edit.apply(timeline)?;
        self.undo.push(inverse);
        Ok(true)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeline::MediaId;

    fn clip(id: u64, start: Frames) -> Clip {
        Clip { id: ClipId(id), media: MediaId(1), start, source_in: 0, duration: 10 }
    }

    #[test]
    fn undo_and_redo_restore_each_state() {
        let mut tl = Timeline::with_default_tracks();
        let mut h = History::default();
        let empty = tl.clone();

        h.apply(&mut tl, Edit::AddClip { track: 0, clip: clip(1, 0) }).unwrap();
        let added = tl.clone();
        h.apply(&mut tl, Edit::MoveClip { track: 0, id: ClipId(1), to: 50 }).unwrap();
        let moved = tl.clone();

        assert!(h.undo(&mut tl).unwrap());
        assert_eq!(tl, added);
        assert!(h.undo(&mut tl).unwrap());
        assert_eq!(tl, empty);
        assert!(!h.undo(&mut tl).unwrap());

        assert!(h.redo(&mut tl).unwrap());
        assert!(h.redo(&mut tl).unwrap());
        assert_eq!(tl, moved);
        assert!(!h.can_redo());
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut tl = Timeline::with_default_tracks();
        let mut h = History::default();
        h.apply(&mut tl, Edit::AddClip { track: 0, clip: clip(1, 0) }).unwrap();
        h.undo(&mut tl).unwrap();
        assert!(h.can_redo());
        h.apply(&mut tl, Edit::AddClip { track: 0, clip: clip(2, 20) }).unwrap();
        assert!(!h.can_redo());
    }

    #[test]
    fn failed_edit_leaves_history_untouched() {
        let mut tl = Timeline::with_default_tracks();
        let mut h = History::default();
        h.apply(&mut tl, Edit::AddClip { track: 0, clip: clip(1, 0) }).unwrap();
        let err = h.apply(&mut tl, Edit::AddClip { track: 0, clip: clip(2, 5) });
        assert_eq!(err, Err(EditError::Overlap));
        assert!(h.undo(&mut tl).unwrap());
        assert!(!h.can_undo());
    }

    #[test]
    fn remove_then_undo_brings_the_same_clip_back() {
        let mut tl = Timeline::with_default_tracks();
        let mut h = History::default();
        h.apply(&mut tl, Edit::AddClip { track: 1, clip: clip(7, 30) }).unwrap();
        h.apply(&mut tl, Edit::RemoveClip { track: 1, id: ClipId(7) }).unwrap();
        assert!(tl.tracks[1].clips().is_empty());
        h.undo(&mut tl).unwrap();
        assert_eq!(tl.tracks[1].clip(ClipId(7)), Some(&clip(7, 30)));
    }
}
