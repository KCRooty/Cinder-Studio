use thiserror::Error;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("no se pudo abrir «{path}»: {reason}")]
    Open { path: String, reason: String },
    #[error("el archivo no tiene vídeo")]
    NoVideo,
    #[error("el archivo no tiene audio")]
    NoAudio,
    #[error("FFmpeg: {0}")]
    Ffmpeg(String),
    #[error("audio: {0}")]
    Audio(String),
    #[error("el hilo de decodificación no arrancó: {0}")]
    Thread(String),
}

impl From<ffmpeg_next::Error> for MediaError {
    fn from(e: ffmpeg_next::Error) -> Self {
        MediaError::Ffmpeg(e.to_string())
    }
}
