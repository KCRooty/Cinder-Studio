//! Operaciones de montaje de la página Edición.
//!
//! Cada operación mira el proyecto y devuelve un [`Edit`] (casi siempre un `Batch`) sin tocar
//! nada; se aplica con [`History::apply`](crate::History::apply), que la hace deshacible de una
//! vez. Así una operación que no cabe falla entera y no deja la línea de tiempo a medias.
//!
//! Las que parten un clip en dos (cuchilla, insertar en medio de un clip, sobrescribir dentro
//! de un clip) reciben el id del trozo nuevo: lo da [`Project::alloc_id`].
//!
//! El ripple de esta fase mueve solo la pista tocada; el ripple sincronizado entre pistas
//! llega con los clips enlazados de vídeo y audio.

use crate::history::Edit;
use crate::project::Project;
use crate::timeline::{Clip, ClipId, EditError, Frames, MediaId, Timeline, Track};

fn track(tl: &Timeline, index: usize) -> Result<&Track, EditError> {
    tl.track(index)
}

fn clip(t: &Track, id: ClipId) -> Result<&Clip, EditError> {
    t.clip(id).ok_or(EditError::NoSuchClip(id))
}

fn media_len(p: &Project, id: MediaId) -> Result<Frames, EditError> {
    p.media.iter().find(|m| m.id == id).map(|m| m.duration).ok_or(EditError::NoSuchMedia(id))
}

/// Comprueba que el clip use solo fotogramas que existen en su medio.
fn check_source(p: &Project, c: &Clip) -> Result<(), EditError> {
    if c.duration <= 0 {
        return Err(EditError::InvalidDuration);
    }
    if c.source_in < 0 || c.source_in + c.duration > media_len(p, c.media)? {
        return Err(EditError::OutOfSource);
    }
    Ok(())
}

/// Las dos mitades de `c` partido en `at` (que tiene que caer dentro, no en un borde).
fn halves(c: &Clip, at: Frames, right_id: ClipId) -> (Clip, Clip) {
    let left = Clip { duration: at - c.start, ..c.clone() };
    let right = Clip {
        id: right_id,
        media: c.media,
        start: at,
        source_in: c.source_in + (at - c.start),
        duration: c.end() - at,
    };
    (left, right)
}

/// Cuchilla: parte en `at` el clip que lo ocupa.
pub fn split(p: &Project, track_index: usize, at: Frames, new_id: ClipId) -> Result<Edit, EditError> {
    let t = track(&p.timeline, track_index)?;
    let c = t.clip_at(at).filter(|c| c.start < at).ok_or(EditError::NothingThere)?;
    let (left, right) = halves(c, at, new_id);
    Ok(Edit::Batch(vec![
        Edit::SetClip { track: track_index, clip: left },
        Edit::AddClip { track: track_index, clip: right },
    ]))
}

/// Borrar con ripple: quita el clip y corre a la izquierda todo lo que venía detrás.
pub fn ripple_delete(p: &Project, track_index: usize, id: ClipId) -> Result<Edit, EditError> {
    let t = track(&p.timeline, track_index)?;
    let c = clip(t, id)?;
    let mut edits = vec![Edit::RemoveClip { track: track_index, id }];
    // En orden ascendente: cada clip cae en un hueco que ya está libre.
    edits.extend(t.clips().iter().filter(|o| o.start >= c.end()).map(|o| Edit::MoveClip {
        track: track_index,
        id: o.id,
        to: o.start - c.duration,
    }));
    Ok(Edit::Batch(edits))
}

/// Insertar: coloca `new` en `new.start` y empuja a la derecha lo que hubiera desde ahí. Si el
/// punto cae dentro de un clip, ese clip se parte y su segunda mitad (con `split_id`) también se
/// empuja.
pub fn insert(p: &Project, track_index: usize, new: Clip, split_id: ClipId) -> Result<Edit, EditError> {
    check_source(p, &new)?;
    if new.start < 0 {
        return Err(EditError::NegativeStart);
    }
    let t = track(&p.timeline, track_index)?;
    let (at, d) = (new.start, new.duration);
    let mut edits = Vec::new();
    let spanning = t.clip_at(at).filter(|c| c.start < at);
    if let Some(c) = spanning {
        edits.push(Edit::SetClip { track: track_index, clip: halves(c, at, split_id).0 });
    }
    // De derecha a izquierda, para que ningún clip pise al siguiente mientras se mueve.
    edits.extend(t.clips().iter().rev().filter(|o| o.start >= at).map(|o| Edit::MoveClip {
        track: track_index,
        id: o.id,
        to: o.start + d,
    }));
    if let Some(c) = spanning {
        let mut right = halves(c, at, split_id).1;
        right.start += d;
        edits.push(Edit::AddClip { track: track_index, clip: right });
    }
    edits.push(Edit::AddClip { track: track_index, clip: new });
    Ok(Edit::Batch(edits))
}

/// Sobrescribir: coloca `new` encima de lo que hubiera, recortando, partiendo (con `split_id`)
/// o quitando lo que tape, sin mover nada más.
pub fn overwrite(p: &Project, track_index: usize, new: Clip, split_id: ClipId) -> Result<Edit, EditError> {
    check_source(p, &new)?;
    if new.start < 0 {
        return Err(EditError::NegativeStart);
    }
    let t = track(&p.timeline, track_index)?;
    let (s, e) = (new.start, new.end());
    let mut edits = Vec::new();
    let mut adds = Vec::new();
    for c in t.clips().iter().filter(|c| c.start < e && s < c.end()) {
        let tr = track_index;
        match (c.start < s, c.end() > e) {
            // Lo tapa por dentro: queda un trozo a cada lado.
            (true, true) => {
                let (left, _) = halves(c, s, split_id);
                let (_, right) = halves(c, e, split_id);
                edits.push(Edit::SetClip { track: tr, clip: left });
                adds.push(Edit::AddClip { track: tr, clip: right });
            }
            // Tapa el final del clip.
            (true, false) => edits.push(Edit::SetClip { track: tr, clip: halves(c, s, c.id).0 }),
            // Tapa el principio.
            (false, true) => {
                let (_, right) = halves(c, e, c.id);
                edits.push(Edit::SetClip { track: tr, clip: right });
            }
            // Lo tapa entero.
            (false, false) => edits.push(Edit::RemoveClip { track: tr, id: c.id }),
        }
    }
    edits.extend(adds);
    edits.push(Edit::AddClip { track: track_index, clip: new });
    Ok(Edit::Batch(edits))
}

/// Recorta el principio del clip `delta` fotogramas (positivo = más corto). El final no se mueve.
pub fn trim_start(p: &Project, track_index: usize, id: ClipId, delta: Frames) -> Result<Edit, EditError> {
    let c = clip(track(&p.timeline, track_index)?, id)?;
    let new =
        Clip { start: c.start + delta, source_in: c.source_in + delta, duration: c.duration - delta, ..c.clone() };
    check_source(p, &new)?;
    Ok(Edit::SetClip { track: track_index, clip: new })
}

/// Recorta el final del clip `delta` fotogramas (positivo = más largo). El principio no se mueve.
pub fn trim_end(p: &Project, track_index: usize, id: ClipId, delta: Frames) -> Result<Edit, EditError> {
    let c = clip(track(&p.timeline, track_index)?, id)?;
    let new = Clip { duration: c.duration + delta, ..c.clone() };
    check_source(p, &new)?;
    Ok(Edit::SetClip { track: track_index, clip: new })
}

/// Slip: cambia qué parte del medio se ve sin mover el clip ni cambiar su duración.
pub fn slip(p: &Project, track_index: usize, id: ClipId, delta: Frames) -> Result<Edit, EditError> {
    let c = clip(track(&p.timeline, track_index)?, id)?;
    let new = Clip { source_in: c.source_in + delta, ..c.clone() };
    check_source(p, &new)?;
    Ok(Edit::SetClip { track: track_index, clip: new })
}

/// Roll: mueve el punto de corte entre `left` y el clip pegado a su derecha. La duración total
/// no cambia: lo que gana uno lo pierde el otro.
pub fn roll(p: &Project, track_index: usize, left: ClipId, delta: Frames) -> Result<Edit, EditError> {
    let t = track(&p.timeline, track_index)?;
    let l = clip(t, left)?;
    let r = t.clips().iter().find(|c| c.start == l.end()).ok_or(EditError::NotAdjacent)?;
    let new_l = Clip { duration: l.duration + delta, ..l.clone() };
    let new_r =
        Clip { start: r.start + delta, source_in: r.source_in + delta, duration: r.duration - delta, ..r.clone() };
    check_source(p, &new_l)?;
    check_source(p, &new_r)?;
    let (a, b) = (Edit::SetClip { track: track_index, clip: new_l }, Edit::SetClip { track: track_index, clip: new_r });
    // Primero encoge el que pierde, para que el otro tenga sitio.
    Ok(Edit::Batch(if delta > 0 { vec![b, a] } else { vec![a, b] }))
}

/// Slide: mueve el clip por la línea de tiempo comiéndose al vecino de un lado y alargando al
/// del otro (si están pegados), sin cambiar lo que se ve del clip ni la duración total.
pub fn slide(p: &Project, track_index: usize, id: ClipId, delta: Frames) -> Result<Edit, EditError> {
    let t = track(&p.timeline, track_index)?;
    let c = clip(t, id)?;
    let prev = t.clips().iter().find(|o| o.end() == c.start);
    let next = t.clips().iter().find(|o| o.start == c.end());
    let moved = Clip { start: c.start + delta, ..c.clone() };
    if moved.start < 0 {
        return Err(EditError::NegativeStart);
    }
    let prev_edit = match prev {
        Some(o) => {
            let n = Clip { duration: o.duration + delta, ..o.clone() };
            check_source(p, &n)?;
            Some(Edit::SetClip { track: track_index, clip: n })
        }
        None => None,
    };
    let next_edit = match next {
        Some(o) => {
            let n = Clip {
                start: o.start + delta,
                source_in: o.source_in + delta,
                duration: o.duration - delta,
                ..o.clone()
            };
            check_source(p, &n)?;
            Some(Edit::SetClip { track: track_index, clip: n })
        }
        None => None,
    };
    let mv = Edit::MoveClip { track: track_index, id, to: moved.start };
    // Hacia la derecha: primero se encoge el de la derecha; hacia la izquierda, al revés.
    let edits = if delta > 0 { [next_edit, Some(mv), prev_edit] } else { [prev_edit, Some(mv), next_edit] };
    Ok(Edit::Batch(edits.into_iter().flatten().collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::History;
    use crate::project::MediaItem;

    const M: MediaId = MediaId(1);

    /// Proyecto con un medio de 1000 fotogramas y, en V1, los clips pedidos (id, inicio, entrada, duración).
    fn project(clips: &[(u64, Frames, Frames, Frames)]) -> Project {
        let mut p = Project::new("ops");
        p.media.push(MediaItem { id: M, name: "m".into(), path: "m.mp4".into(), duration: 1000 });
        for &(id, start, source_in, duration) in clips {
            p.timeline.tracks[0].insert(Clip { id: ClipId(id), media: M, start, source_in, duration }).unwrap();
        }
        p
    }

    /// (id, inicio, entrada, duración) de cada clip de V1.
    fn layout(p: &Project) -> Vec<(u64, Frames, Frames, Frames)> {
        p.timeline.tracks[0].clips().iter().map(|c| (c.id.0, c.start, c.source_in, c.duration)).collect()
    }

    /// Aplica la operación, comprueba el resultado y que deshacer deja todo como estaba.
    fn check(mut p: Project, edit: Result<Edit, EditError>, want: &[(u64, Frames, Frames, Frames)]) {
        let before = p.timeline.clone();
        let mut h = History::default();
        h.apply(&mut p.timeline, edit.unwrap()).unwrap();
        assert_eq!(layout(&p), want);
        h.undo(&mut p.timeline).unwrap();
        assert_eq!(p.timeline, before, "deshacer no dejó la línea de tiempo igual");
        h.redo(&mut p.timeline).unwrap();
        assert_eq!(layout(&p), want, "rehacer no repitió lo mismo");
    }

    fn new_clip(id: u64, start: Frames, source_in: Frames, duration: Frames) -> Clip {
        Clip { id: ClipId(id), media: M, start, source_in, duration }
    }

    #[test]
    fn cuchilla_parte_el_clip_y_conserva_la_imagen() {
        let p = project(&[(1, 10, 100, 50)]);
        check(p.clone(), split(&p, 0, 30, ClipId(9)), &[(1, 10, 100, 20), (9, 30, 120, 30)]);
    }

    #[test]
    fn cuchilla_en_un_borde_o_en_el_vacio_no_hace_nada() {
        let p = project(&[(1, 10, 0, 50)]);
        assert_eq!(split(&p, 0, 10, ClipId(9)), Err(EditError::NothingThere));
        assert_eq!(split(&p, 0, 60, ClipId(9)), Err(EditError::NothingThere));
        assert_eq!(split(&p, 0, 5, ClipId(9)), Err(EditError::NothingThere));
    }

    #[test]
    fn ripple_delete_cierra_el_hueco() {
        let p = project(&[(1, 0, 0, 10), (2, 10, 0, 20), (3, 30, 0, 5), (4, 50, 0, 5)]);
        check(p.clone(), ripple_delete(&p, 0, ClipId(2)), &[(1, 0, 0, 10), (3, 10, 0, 5), (4, 30, 0, 5)]);
    }

    #[test]
    fn insertar_en_un_hueco_empuja_lo_de_detras() {
        let p = project(&[(1, 0, 0, 10), (2, 20, 0, 10)]);
        check(
            p.clone(),
            insert(&p, 0, new_clip(5, 10, 500, 4), ClipId(9)),
            &[(1, 0, 0, 10), (5, 10, 500, 4), (2, 24, 0, 10)],
        );
    }

    #[test]
    fn insertar_dentro_de_un_clip_lo_parte() {
        let p = project(&[(1, 0, 100, 30), (2, 30, 0, 10)]);
        check(
            p.clone(),
            insert(&p, 0, new_clip(5, 10, 500, 5), ClipId(9)),
            &[(1, 0, 100, 10), (5, 10, 500, 5), (9, 15, 110, 20), (2, 35, 0, 10)],
        );
    }

    #[test]
    fn sobrescribir_recorta_parte_y_quita_lo_que_tapa() {
        // Clip 1 tapado por el final, 2 entero, 3 por el principio.
        let p = project(&[(1, 0, 0, 10), (2, 10, 0, 10), (3, 20, 0, 10)]);
        check(
            p.clone(),
            overwrite(&p, 0, new_clip(5, 5, 500, 20), ClipId(9)),
            &[(1, 0, 0, 5), (5, 5, 500, 20), (3, 25, 5, 5)],
        );
        // Dentro de un solo clip: quedan los dos lados.
        let p = project(&[(1, 0, 100, 30)]);
        check(
            p.clone(),
            overwrite(&p, 0, new_clip(5, 10, 500, 5), ClipId(9)),
            &[(1, 0, 100, 10), (5, 10, 500, 5), (9, 15, 115, 15)],
        );
    }

    #[test]
    fn recortes_respetan_el_medio_y_los_vecinos() {
        let p = project(&[(1, 10, 5, 20), (2, 30, 0, 10)]);
        check(p.clone(), trim_start(&p, 0, ClipId(1), 3), &[(1, 13, 8, 17), (2, 30, 0, 10)]);
        check(p.clone(), trim_start(&p, 0, ClipId(1), -5), &[(1, 5, 0, 25), (2, 30, 0, 10)]);
        assert_eq!(trim_start(&p, 0, ClipId(1), -6), Err(EditError::OutOfSource));
        check(p.clone(), trim_end(&p, 0, ClipId(1), -4), &[(1, 10, 5, 16), (2, 30, 0, 10)]);
        // Alargar el final chocaría con el clip 2: falla al aplicarlo y no cambia nada.
        let mut q = p.clone();
        let before = q.timeline.clone();
        let err = History::default().apply(&mut q.timeline, trim_end(&p, 0, ClipId(1), 1).unwrap());
        assert_eq!(err, Err(EditError::Overlap));
        assert_eq!(q.timeline, before);
    }

    #[test]
    fn slip_cambia_la_imagen_sin_mover_el_clip() {
        let p = project(&[(1, 10, 100, 20)]);
        check(p.clone(), slip(&p, 0, ClipId(1), 30), &[(1, 10, 130, 20)]);
        assert_eq!(slip(&p, 0, ClipId(1), -101), Err(EditError::OutOfSource));
        assert_eq!(slip(&p, 0, ClipId(1), 881), Err(EditError::OutOfSource));
    }

    #[test]
    fn roll_mueve_el_corte_entre_dos_clips() {
        let p = project(&[(1, 0, 100, 20), (2, 20, 200, 20)]);
        check(p.clone(), roll(&p, 0, ClipId(1), 5), &[(1, 0, 100, 25), (2, 25, 205, 15)]);
        check(p.clone(), roll(&p, 0, ClipId(1), -5), &[(1, 0, 100, 15), (2, 15, 195, 25)]);
        let gap = project(&[(1, 0, 0, 10), (2, 15, 0, 10)]);
        assert_eq!(roll(&gap, 0, ClipId(1), 1), Err(EditError::NotAdjacent));
    }

    #[test]
    fn slide_mueve_el_clip_entre_sus_vecinos() {
        let p = project(&[(1, 0, 100, 20), (2, 20, 300, 10), (3, 30, 200, 20)]);
        check(p.clone(), slide(&p, 0, ClipId(2), 4), &[(1, 0, 100, 24), (2, 24, 300, 10), (3, 34, 204, 16)]);
        check(p.clone(), slide(&p, 0, ClipId(2), -4), &[(1, 0, 100, 16), (2, 16, 300, 10), (3, 26, 196, 24)]);
    }

    #[test]
    fn una_operacion_que_falla_a_medias_no_deja_rastro() {
        // El clip nuevo se sale del medio: ni siquiera llega a aplicarse.
        let p = project(&[(1, 0, 0, 10)]);
        assert_eq!(insert(&p, 0, new_clip(5, 0, 990, 20), ClipId(9)), Err(EditError::OutOfSource));
        // Un lote que choca en el último paso deshace los anteriores.
        let mut q = project(&[(1, 0, 0, 10), (2, 20, 0, 10)]);
        let before = q.timeline.clone();
        let bad = Edit::Batch(vec![
            Edit::MoveClip { track: 0, id: ClipId(2), to: 40 },
            Edit::AddClip { track: 0, clip: new_clip(5, 5, 0, 10) },
        ]);
        assert_eq!(History::default().apply(&mut q.timeline, bad), Err(EditError::Overlap));
        assert_eq!(q.timeline, before);
    }
}
