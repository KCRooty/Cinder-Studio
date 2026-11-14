//! Medios de Cinder Studio: abrir un clip, decodificarlo con FFmpeg en su propio hilo,
//! sacar el audio por la tarjeta de sonido y llevar el reloj de reproducción.
//!
//! El reloj maestro es el audio (ver `docs/ARQUITECTURA.md`): el visor pide el fotograma
//! que toca según [`Player::position`] y salta los que llegan tarde.

mod audio;
mod decoder;
mod error;
mod player;

pub use error::MediaError;
pub use player::{MediaInfo, Player, VideoFrame};

/// Prepara FFmpeg. Se puede llamar varias veces.
pub fn init() -> Result<(), MediaError> {
    ffmpeg_next::init().map_err(|e| MediaError::Ffmpeg(e.to_string()))
}
