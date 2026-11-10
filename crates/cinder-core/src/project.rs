use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::timeline::{Frames, MediaId, Rational, Timeline};

/// Versión del formato `.cinder`. Sube con cada cambio incompatible, y cada subida trae su migración.
pub const FORMAT_VERSION: u32 = 1;

/// Archivo principal dentro de la carpeta `.cinder`.
pub const PROJECT_FILE: &str = "proyecto.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSettings {
    pub fps: Rational,
    pub width: u32,
    pub height: u32,
    pub sample_rate: u32,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self { fps: Rational::new(30, 1), width: 1920, height: 1080, sample_rate: 48_000 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaItem {
    pub id: MediaId,
    pub name: String,
    /// Relativa a la carpeta del proyecto cuando el medio está dentro de ella.
    pub path: PathBuf,
    pub duration: Frames,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub version: u32,
    pub name: String,
    pub settings: ProjectSettings,
    pub media: Vec<MediaItem>,
    pub timeline: Timeline,
    next_id: u64,
}

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("no se pudo leer o escribir {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("el proyecto está dañado: {0}")]
    Parse(#[from] serde_json::Error),
    #[error(
        "el proyecto es de una versión más nueva de Cinder Studio (formato {found}; esta versión entiende hasta el {supported})"
    )]
    TooNew { found: u32, supported: u32 },
    #[error("formato de proyecto desconocido ({0})")]
    Unsupported(u32),
}

#[derive(Deserialize)]
struct Header {
    version: u32,
}

impl Project {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            version: FORMAT_VERSION,
            name: name.into(),
            settings: ProjectSettings::default(),
            media: Vec::new(),
            timeline: Timeline::with_default_tracks(),
            next_id: 1,
        }
    }

    /// Identificador nuevo, único dentro del proyecto, para medios y clips.
    pub fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Guarda en `dir` (la carpeta `.cinder`), creándola si hace falta.
    /// Escribe a un archivo temporal y lo renombra, para no dejar un proyecto a medias si algo falla.
    pub fn save(&self, dir: &Path) -> Result<(), ProjectError> {
        fs::create_dir_all(dir).map_err(|source| ProjectError::Io { path: dir.to_path_buf(), source })?;
        let json = serde_json::to_string_pretty(self)?;
        let path = dir.join(PROJECT_FILE);
        let tmp = dir.join(format!("{PROJECT_FILE}.tmp"));
        fs::write(&tmp, json).map_err(|source| ProjectError::Io { path: tmp.clone(), source })?;
        fs::rename(&tmp, &path).map_err(|source| ProjectError::Io { path, source })
    }

    pub fn load(dir: &Path) -> Result<Self, ProjectError> {
        let path = dir.join(PROJECT_FILE);
        let text = fs::read_to_string(&path).map_err(|source| ProjectError::Io { path, source })?;
        Self::from_json(&text)
    }

    pub fn from_json(text: &str) -> Result<Self, ProjectError> {
        let Header { version } = serde_json::from_str(text)?;
        match version {
            FORMAT_VERSION => Ok(serde_json::from_str(text)?),
            v if v > FORMAT_VERSION => Err(ProjectError::TooNew { found: v, supported: FORMAT_VERSION }),
            v => Err(ProjectError::Unsupported(v)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{Edit, History};
    use crate::timeline::{Clip, ClipId};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cinder-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn sample() -> Project {
        let mut p = Project::new("Prueba");
        let media = MediaId(p.alloc_id());
        p.media.push(MediaItem { id: media, name: "toma1.mp4".into(), path: "medios/toma1.mp4".into(), duration: 300 });
        let clip = Clip { id: ClipId(p.alloc_id()), media, start: 0, source_in: 12, duration: 120 };
        History::default().apply(&mut p.timeline, Edit::AddClip { track: 0, clip }).unwrap();
        p
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = temp_dir("roundtrip").join("Prueba.cinder");
        let p = sample();
        p.save(&dir).unwrap();
        assert!(!dir.join(format!("{PROJECT_FILE}.tmp")).exists());
        assert_eq!(Project::load(&dir).unwrap(), p);
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn ids_keep_counting_after_reload() {
        let dir = temp_dir("ids").join("p.cinder");
        let mut p = sample();
        let before = p.alloc_id();
        p.save(&dir).unwrap();
        let mut loaded = Project::load(&dir).unwrap();
        assert_eq!(loaded.alloc_id(), before + 1);
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn newer_format_is_refused_with_a_clear_error() {
        let json = serde_json::to_string(&sample()).unwrap().replacen("\"version\":1", "\"version\":99", 1);
        match Project::from_json(&json) {
            Err(ProjectError::TooNew { found: 99, supported: FORMAT_VERSION }) => {}
            other => panic!("esperaba TooNew, llegó {other:?}"),
        }
    }

    #[test]
    fn garbage_is_a_parse_error() {
        assert!(matches!(Project::from_json("{nope"), Err(ProjectError::Parse(_))));
    }

    #[test]
    fn missing_project_is_an_io_error_naming_the_file() {
        let err = Project::load(&temp_dir("missing")).unwrap_err();
        assert!(err.to_string().contains(PROJECT_FILE));
    }
}
