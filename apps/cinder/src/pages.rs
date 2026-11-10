use eframe::egui::Key;

/// Las siete páginas, en el orden de la barra inferior (ver `docs/PAGINAS.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Medios,
    Corte,
    Edicion,
    Composicion,
    Color,
    Audio,
    Entrega,
}

impl Page {
    pub const ALL: [Page; 7] =
        [Page::Medios, Page::Corte, Page::Edicion, Page::Composicion, Page::Color, Page::Audio, Page::Entrega];

    pub fn label(self) -> &'static str {
        match self {
            Page::Medios => "Medios",
            Page::Corte => "Corte",
            Page::Edicion => "Edición",
            Page::Composicion => "Composición",
            Page::Color => "Color",
            Page::Audio => "Audio",
            Page::Entrega => "Entrega",
        }
    }

    /// Mayús+2 … Mayús+8, el mismo atajo para cambiar de página que en Resolve.
    pub fn key(self) -> Key {
        match self {
            Page::Medios => Key::Num2,
            Page::Corte => Key::Num3,
            Page::Edicion => Key::Num4,
            Page::Composicion => Key::Num5,
            Page::Color => Key::Num6,
            Page::Audio => Key::Num7,
            Page::Entrega => Key::Num8,
        }
    }

    pub fn shortcut_label(self) -> &'static str {
        match self {
            Page::Medios => "Mayús+2",
            Page::Corte => "Mayús+3",
            Page::Edicion => "Mayús+4",
            Page::Composicion => "Mayús+5",
            Page::Color => "Mayús+6",
            Page::Audio => "Mayús+7",
            Page::Entrega => "Mayús+8",
        }
    }

    pub fn summary(self) -> &'static str {
        match self {
            Page::Medios => "Importar clips, organizarlos en carpetas, ver metadatos y generar proxies.",
            Page::Corte => "Montaje rápido con línea de tiempo doble: general y detalle.",
            Page::Edicion => "Edición completa: pistas, recortes ripple/roll/slip/slide, transiciones y títulos.",
            Page::Composicion => "Composición nodal: fusiones, máscaras, keyer, tracker y partículas.",
            Page::Color => "Corrección de color por nodos: ruedas, curvas, cualificadores, ventanas, LUTs y scopes.",
            Page::Audio => "Mezcla multipista: buses, EQ y dinámica por pista, automatización y plugins CLAP.",
            Page::Entrega => "Cola de render y presets de exportación.",
        }
    }

    /// Fase de `docs/ROADMAP.md` en la que entra la página.
    pub fn phase(self) -> u8 {
        match self {
            Page::Medios | Page::Edicion => 2,
            Page::Entrega => 3,
            Page::Color => 4,
            Page::Audio => 5,
            Page::Composicion => 6,
            Page::Corte => 7,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_has_its_own_shortcut() {
        let mut keys: Vec<_> = Page::ALL.iter().map(|p| p.key()).collect();
        keys.dedup();
        assert_eq!(keys.len(), Page::ALL.len());
    }
}
