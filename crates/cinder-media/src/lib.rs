//! Medios de Cinder Studio: abrir un clip, decodificarlo con FFmpeg en su propio hilo,
//! sacar el audio por la tarjeta de sonido y llevar el reloj de reproducción.
//!
//! El reloj maestro es el audio (ver `docs/ARQUITECTURA.md`): el visor pide el fotograma
//! que toca según [`Player::position`] y salta los que llegan tarde.

mod audio;
mod audio_clip;
mod decoder;
mod error;
mod mixer;
mod player;
mod thumb;

pub use audio_clip::peaks;
pub use error::MediaError;
pub use mixer::{AudioClipHandle, Mixer};
pub use player::{MediaInfo, Player, VideoFrame};
pub use thumb::thumbnail;

/// Prepara FFmpeg. Se puede llamar varias veces.
pub fn init() -> Result<(), MediaError> {
    ffmpeg_next::init().map_err(|e| MediaError::Ffmpeg(e.to_string()))
}
