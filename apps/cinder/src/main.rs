#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod pages;

use cinder_core::Project;
use eframe::egui::{self, Color32, RichText};

use pages::Page;

const ACCENT: Color32 = Color32::from_rgb(0x7B, 0x61, 0xFF);

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Cinder Studio")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([960.0, 600.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native("Cinder Studio", options, Box::new(|cc| Ok(Box::new(CinderApp::new(cc)))))
}

struct CinderApp {
    project: Project,
    page: Page,
}

impl CinderApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.selection.bg_fill = ACCENT;
        visuals.hyperlink_color = ACCENT;
        cc.egui_ctx.set_visuals(visuals);
        Self { project: Project::new("Proyecto sin título"), page: Page::Edicion }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        // Con Mayús, la tecla 6 llega como «&» (y así en cada distribución); se mira la tecla física.
        let pressed = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, physical_key, pressed: true, repeat: false, modifiers, .. }
                    if modifiers.shift =>
                {
                    let k = physical_key.unwrap_or(*key);
                    Page::ALL.into_iter().find(|p| p.key() == k)
                }
                _ => None,
            })
        });
        if let Some(page) = pressed {
            self.page = page;
        }
    }

    fn top_bar(&self, ui: &mut egui::Ui) {
        let s = &self.project.settings;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Cinder Studio").strong().color(ACCENT));
            ui.separator();
            ui.label(&self.project.name);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(format!("{}×{} · {:.3} fps · {} Hz", s.width, s.height, s.fps.as_f64(), s.sample_rate));
            });
        });
    }

    fn page_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            for page in Page::ALL {
                let text = RichText::new(page.label()).size(15.0);
                if ui.selectable_label(self.page == page, text).on_hover_text(page.shortcut_label()).clicked() {
                    self.page = page;
                }
            }
        });
    }

    fn page_body(&self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.3);
            ui.heading(RichText::new(self.page.label()).size(32.0));
            ui.add_space(8.0);
            ui.label(self.page.summary());
            ui.add_space(4.0);
            ui.weak(format!("Esta página llega en la fase {} del roadmap.", self.page.phase()));
        });
    }
}

impl eframe::App for CinderApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_shortcuts(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("barra-superior").show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("barra-paginas").exact_size(44.0).show(ui, |ui| self.page_bar(ui));
        egui::CentralPanel::default().show(ui, |ui| self.page_body(ui));
    }
}
