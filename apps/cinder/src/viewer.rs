//! Visor de un clip con su barra de transporte (Fase 1: un clip suelto, sin línea de tiempo).
//!
//! Atajos (solo con el visor visible y sin un campo de texto activo):
//! Espacio reproduce o para · K para · L reproduce · J retrocede 1 s (la reproducción hacia
//! atrás llega con la línea de tiempo, en la Fase 2) · ← / → un fotograma · Inicio / Fin.

use std::path::{Path, PathBuf};

use cinder_media::Player;
use eframe::egui::{self, Color32, ColorImage, Key, RichText, TextureHandle, TextureOptions, Vec2};

pub struct Viewer {
    player: Option<Player>,
    texture: Option<TextureHandle>,
    /// pts del fotograma que está en la textura, para no subirlo otra vez.
    shown_pts: Option<f64>,
    error: Option<String>,
}

impl Viewer {
    pub fn new() -> Self {
        Self { player: None, texture: None, shown_pts: None, error: None }
    }

    pub fn open(&mut self, path: &Path) {
        // Primero se suelta el reproductor viejo: su hilo y su audio se cierran antes.
        self.player = None;
        self.texture = None;
        self.shown_pts = None;
        match Player::open(path) {
            Ok(p) => {
                self.error = None;
                self.player = Some(p);
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn pick_file(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("Abrir clip")
            .add_filter("Vídeo", &["mp4", "mov", "mkv", "webm", "avi", "m4v", "mxf", "mts", "ts"])
            .add_filter("Todos los archivos", &["*"])
            .pick_file();
        if let Some(path) = picked {
            self.open(&path);
        }
    }

    pub fn handle_input(&mut self, ctx: &egui::Context) {
        let dropped: Option<PathBuf> = ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()));
        if let Some(path) = dropped {
            self.open(&path);
        }
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let ctrl_o = ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::O));
        if ctrl_o {
            self.pick_file();
        }
        let Some(p) = self.player.as_mut() else { return };
        let step = 1.0 / if p.info().fps > 0.0 { p.info().fps } else { 25.0 };
        ctx.input(|i| {
            if i.modifiers.any() {
                return;
            }
            if i.key_pressed(Key::Space) {
                p.toggle();
            }
            if i.key_pressed(Key::K) {
                p.pause();
            }
            if i.key_pressed(Key::L) {
                p.play();
            }
            if i.key_pressed(Key::J) {
                let t = p.position();
                p.seek(t - 1.0);
            }
            if i.key_pressed(Key::ArrowRight) {
                p.pause();
                let t = p.position();
                p.seek(t + step);
            }
            if i.key_pressed(Key::ArrowLeft) {
                p.pause();
                let t = p.position();
                p.seek(t - step);
            }
            if i.key_pressed(Key::Home) {
                p.seek(0.0);
            }
            if i.key_pressed(Key::End) {
                p.pause();
                let d = p.info().duration;
                p.seek(d - step);
            }
        });
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if self.player.is_none() {
            self.empty_state(ui);
            return;
        }
        self.upload_frame(ui.ctx());
        let transport_h = 64.0;
        let area = ui.available_size() - Vec2::new(0.0, transport_h);
        self.picture(ui, area);
        ui.add_space(6.0);
        self.transport(ui);
    }

    fn empty_state(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.3);
            ui.heading(RichText::new("Visor").size(28.0));
            ui.add_space(6.0);
            ui.label("Arrastra un clip a la ventana o ábrelo con Ctrl+O.");
            ui.add_space(10.0);
            if ui.button(RichText::new("Abrir clip…").size(15.0)).clicked() {
                self.pick_file();
            }
            if let Some(e) = &self.error {
                ui.add_space(10.0);
                ui.colored_label(Color32::from_rgb(0xF0, 0x7F, 0x7F), e);
            }
        });
    }

    fn upload_frame(&mut self, ctx: &egui::Context) {
        let Some(p) = self.player.as_mut() else { return };
        let playing = p.is_playing();
        if let Some(frame) = p.frame()
            && self.shown_pts != Some(frame.pts)
        {
            let image = ColorImage::from_rgba_unmultiplied([frame.width as usize, frame.height as usize], &frame.rgba);
            match self.texture.as_mut() {
                Some(t) => t.set(image, TextureOptions::LINEAR),
                None => self.texture = Some(ctx.load_texture("visor", image, TextureOptions::LINEAR)),
            }
            self.shown_pts = Some(frame.pts);
        }
        // Reproduciendo: repintar a cada vuelta. En pausa: lo justo para que llegue el
        // fotograma de un salto.
        if playing {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
    }

    fn picture(&self, ui: &mut egui::Ui, area: Vec2) {
        let (rect, _) = ui.allocate_exact_size(area.max(Vec2::splat(1.0)), egui::Sense::hover());
        ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
        let Some(tex) = &self.texture else { return };
        let size = tex.size_vec2();
        if size.x <= 0.0 || size.y <= 0.0 {
            return;
        }
        let scale = (rect.width() / size.x).min(rect.height() / size.y);
        let fit = egui::Rect::from_center_size(rect.center(), size * scale);
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.painter().image(tex.id(), fit, uv, Color32::WHITE);
    }

    fn transport(&mut self, ui: &mut egui::Ui) {
        let Some(p) = self.player.as_mut() else { return };
        let info = p.info().clone();
        let mut pos = p.position();
        ui.horizontal(|ui| {
            if play_button(ui, p.is_playing()).on_hover_text("Espacio · K / L").clicked() {
                p.toggle();
            }
            ui.monospace(timecode(pos, info.fps));
            let width = (ui.available_width() - 120.0).max(60.0);
            ui.spacing_mut().slider_width = width;
            let slider = egui::Slider::new(&mut pos, 0.0..=info.duration.max(0.001)).show_value(false);
            if ui.add(slider).changed() {
                p.seek(pos);
            }
            ui.monospace(timecode(info.duration, info.fps));
        });
        ui.horizontal(|ui| {
            let name = info.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            ui.weak(format!(
                "{name} · {}×{} · {:.3} fps · {}{}",
                info.width,
                info.height,
                info.fps,
                info.video_codec,
                info.audio_codec.as_deref().map(|a| format!(" + {a}")).unwrap_or_default()
            ));
            if let Some(e) = p.audio_error() {
                ui.colored_label(Color32::from_rgb(0xF5, 0xA6, 0x23), format!("sin sonido: {e}"));
            }
        });
    }
}

/// Botón de reproducir/pausa dibujado (las fuentes por defecto no traen ▶ ni ⏸).
fn play_button(ui: &mut egui::Ui, playing: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(28.0), egui::Sense::click());
    let visuals = ui.style().interact(&resp);
    ui.painter().rect_filled(rect, 4.0, visuals.weak_bg_fill);
    let c = rect.center();
    let color = visuals.fg_stroke.color;
    if playing {
        for dx in [-3.5, 3.5] {
            let bar = egui::Rect::from_center_size(egui::pos2(c.x + dx, c.y), Vec2::new(3.5, 12.0));
            ui.painter().rect_filled(bar, 1.0, color);
        }
    } else {
        let pts = vec![egui::pos2(c.x - 4.0, c.y - 7.0), egui::pos2(c.x - 4.0, c.y + 7.0), egui::pos2(c.x + 7.0, c.y)];
        ui.painter().add(egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE));
    }
    resp
}

/// HH:MM:SS:FF, como en cualquier editor.
pub fn timecode(secs: f64, fps: f64) -> String {
    let fps = if fps > 0.0 { fps } else { 25.0 };
    let base = fps.round().max(1.0) as u64;
    let total = (secs.max(0.0) * fps).round() as u64;
    let f = total % base;
    let s = total / base;
    format!("{:02}:{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60, f)
}

#[cfg(test)]
mod tests {
    use super::timecode;

    #[test]
    fn timecode_cuenta_fotogramas() {
        assert_eq!(timecode(0.0, 25.0), "00:00:00:00");
        assert_eq!(timecode(1.24, 25.0), "00:00:01:06");
        assert_eq!(timecode(3661.48, 25.0), "01:01:01:12");
        assert_eq!(timecode(2.0, 0.0), "00:00:02:00");
    }
}
