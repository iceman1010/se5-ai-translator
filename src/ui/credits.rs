use eframe::egui;

use super::TranslatorApp;

impl TranslatorApp {
    pub fn draw_credits_tab(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.heading("Credits");
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Coming soon").color(egui::Color32::GRAY));
        });
    }
}
