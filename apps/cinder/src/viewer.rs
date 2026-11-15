//! Visor de origen: un clip suelto con su barra de transporte y sus marcas de entrada y salida.
//!
//! Atajos (con el visor activo y sin un campo de texto activo):
//! Espacio reproduce o para · K para · L reproduce · J retrocede 1 s · ← / → un fotograma ·
//! Inicio / Fin · I / O marcan entrada y salida · Alt+X las quita.

use std::path::{Path, PathBuf};

use cinder_media::Player;
use eframe::egui::{self, Color32, ColorImage, Key, RichText, TextureHandle, TextureOptions, Vec2};

pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "mkv", "webm", "avi", "m4v", "mxf", "mts", "ts"];

pub struct Viewer {
    player: Option<Player>,
    texture: Option<TextureHandle>,
    /// pts del fotograma que está en la textura, para no subirlo otra vez.
    shown_pts: Option<f64>,
    error: Option<String>,
    /// Entrada y salida marcadas, en segundos del clip.
    mark_in: Option<f64>,
    mark_out: Option<f64>,
    /// Se pidió importar desde el visor vacío; lo atiende la app.
    pub wants_import: bool,
}

impl Viewer {
    pub fn new() -> Self {
        Self {
            player: None,
            texture: None,
            shown_pts: None,
            error: None,
            mark_in: None,
            mark_out: None,
            wants_import: false,
        }
    }

    pub fn open(&mut self, path: &Path) {
        // Primero se suelta el reproductor viejo: su hilo y su audio se cierran antes.
        self.player = None;
        self.texture = None;
        self.shown_pts = None;
        self.mark_in = None;
        self.mark_out = None;
        match Player::open(path) {
            Ok(p) => {
                self.error = None;
                self.player = Some(p);
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.player.as_ref().map(|p| p.info().path.as_path())
    }

    pub fn info(&self) -> Option<&cinder_media::MediaInfo> {
        self.player.as_ref().map(Player::info)
    }

    /// Tramo marcado en segundos: de la entrada (o el principio) a la salida (o el final).
    pub fn marked_range(&self) -> Option<(f64, f64)> {
        let info = self.info()?;
        let a = self.mark_in.unwrap_or(0.0);
        let b = self.mark_out.unwrap_or(info.duration);
        (b > a).then_some((a, b))
    }

    pub fn pause(&mut self) {
        if let Some(p) = self.player.as_mut() {
            p.pause();
        }
    }

    /// Rutas elegidas en el diálogo de importar.
    pub fn pick_paths() -> Vec<PathBuf> {
        rfd::FileDialog::new()
            .set_title("Importar clips")
            .add_filter("Vídeo", VIDEO_EXTENSIONS)
            .add_filter("Todos los archivos", &["*"])
            .pick_files()
            .unwrap_or_default()
    }

    pub fn handle_input(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let Some(p) = self.player.as_mut() else { return };
        let step = 1.0 / if p.info().fps > 0.0 { p.info().fps } else { 25.0 };
        let (mark_in, mark_out) = (&mut self.mark_in, &mut self.mark_out);
        ctx.input(|i| {
            if i.modifiers.alt && i.key_pressed(Key::X) {
                *mark_in = None;
                *mark_out = None;
            }
            if i.modifiers.any() {
                return;
            }
            if i.key_pressed(Key::I) {
                *mark_in = Some(p.position());
                if mark_out.is_some_and(|o| o <= p.position()) {
                    *mark_out = None;
                }
            }
            if i.key_pressed(Key::O) {
                *mark_out = Some(p.position());
                if mark_in.is_some_and(|m| m >= p.position()) {
                    *mark_in = None;
                }
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

    /// Devuelve `true` si se hizo clic en la imagen (para que pase a ser el visor activo).
    pub fn ui(&mut self, ui: &mut egui::Ui) -> bool {
        if self.player.is_none() {
            self.empty_state(ui);
            return false;
        }
        self.upload_frame(ui.ctx());
        let transport_h = 64.0;
        let area = ui.available_size() - Vec2::new(0.0, transport_h);
        let clicked = draw_picture(ui, self.texture.as_ref(), area).clicked();
        ui.add_space(6.0);
        self.transport(ui);
        clicked
    }

    fn empty_state(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.3);
            ui.heading(RichText::new("Visor").size(28.0));
            ui.add_space(6.0);
            ui.label("Arrastra clips a la ventana o impórtalos con Ctrl+I.");
            ui.add_space(10.0);
            if ui.button(RichText::new("Importar clips…").size(15.0)).clicked() {
                self.wants_import = true;
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
            upload(ctx, &mut self.texture, "visor-origen", frame);
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
            let total = info.duration.max(0.001);
            let slider = egui::Slider::new(&mut pos, 0.0..=total).show_value(false);
            let resp = ui.add(slider);
            if resp.changed() {
                p.seek(pos);
            }
            draw_marks(ui, resp.rect, total, self.mark_in, self.mark_out);
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

/// Sube un fotograma a la textura (la crea la primera vez).
pub fn upload(ctx: &egui::Context, texture: &mut Option<TextureHandle>, name: &str, frame: &cinder_media::VideoFrame) {
    let image = ColorImage::from_rgba_unmultiplied([frame.width as usize, frame.height as usize], &frame.rgba);
    match texture.as_mut() {
        Some(t) => t.set(image, TextureOptions::LINEAR),
        None => *texture = Some(ctx.load_texture(name, image, TextureOptions::LINEAR)),
    }
}

/// Fondo negro y la imagen encajada sin deformar.
pub fn draw_picture(ui: &mut egui::Ui, texture: Option<&TextureHandle>, area: Vec2) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(area.max(Vec2::splat(1.0)), egui::Sense::click());
    ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
    let Some(tex) = texture else { return resp };
    let size = tex.size_vec2();
    if size.x <= 0.0 || size.y <= 0.0 {
        return resp;
    }
    let scale = (rect.width() / size.x).min(rect.height() / size.y);
    let fit = egui::Rect::from_center_size(rect.center(), size * scale);
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    ui.painter().image(tex.id(), fit, uv, Color32::WHITE);
    resp
}

/// Tramo entre entrada y salida pintado bajo la barra de posición.
fn draw_marks(ui: &egui::Ui, rect: egui::Rect, total: f64, mark_in: Option<f64>, mark_out: Option<f64>) {
    if mark_in.is_none() && mark_out.is_none() {
        return;
    }
    let x = |t: f64| rect.left() + rect.width() * (t / total).clamp(0.0, 1.0) as f32;
    let (a, b) = (x(mark_in.unwrap_or(0.0)), x(mark_out.unwrap_or(total)));
    let band = egui::Rect::from_min_max(egui::pos2(a, rect.bottom() - 3.0), egui::pos2(b, rect.bottom()));
    ui.painter().rect_filled(band, 0.0, crate::ACCENT);
}

/// Botón de reproducir/pausa dibujado (las fuentes por defecto no traen ▶ ni ⏸).
pub fn play_button(ui: &mut egui::Ui, playing: bool) -> egui::Response {
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
