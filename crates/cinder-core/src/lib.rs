//! Modelo de proyecto de Cinder Studio.
//!
//! Todo lo que las páginas editan vive aquí: medios, línea de tiempo y ajustes,
//! con deshacer/rehacer por comandos y guardado en el formato `.cinder`.

pub mod history;
pub mod project;
pub mod timeline;

pub use history::{Edit, History};
pub use project::{FORMAT_VERSION, MediaItem, Project, ProjectError, ProjectSettings};
pub use timeline::{Clip, ClipId, EditError, Frames, MediaId, Rational, Timeline, Track, TrackKind};
