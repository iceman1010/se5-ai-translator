use crate::api::{ApiClient, LanguageInfo};
use eframe::egui;
use std::sync::atomic::Ordering;

use super::TranslatorApp;

pub struct ThreadResult {
    pub translated: Option<String>,
    pub error: Option<String>,
}

pub struct DetectResult {
    pub iso_code: String,
    pub w3c_code: String,
    pub language_name: String,
}

pub fn find_nearest_language_idx(languages: &[LanguageInfo], detected_code: &str) -> Option<usize> {
    let detected_lower = detected_code.to_lowercase();

    languages.iter().position(|l| l.language_code.to_lowercase() == detected_lower).or_else(|| {
        let base_detected = detected_lower.split('-').next().unwrap_or(&detected_lower);
        let base_detected2 = detected_lower.split('_').next().unwrap_or(&detected_lower);
        languages.iter().position(|l| {
            let lc_lower = l.language_code.to_lowercase();
            let base_lang = lc_lower.split('-').next().unwrap_or("");
            let base_lang2 = lc_lower.split('_').next().unwrap_or("");
            base_lang == base_detected || base_lang2 == base_detected2
                || base_detected == base_lang || base_detected2 == base_lang2
        })
    })
}

impl TranslatorApp {
    pub fn fetch_engines_and_languages(&mut self) {
        let auth_token = self.settings.auth_token.clone().unwrap_or_default();

        let client = ApiClient::new(&auth_token);
        self.engines = match client.fetch_engines() {
            Ok(e) => e,
            Err(e) => {
                self.translate_status = format!("Failed to load engines: {e}");
                return;
            }
        };

        if !self.engines.is_empty() {
            if let Some(ref last_engine) = self.settings.last_engine {
                self.selected_engine_idx = self
                    .engines
                    .iter()
                    .position(|e| e == last_engine)
                    .unwrap_or(0);
            }
        }

        let engine_for_langs = self.engines.get(self.selected_engine_idx).map(|s| s.as_str());
        self.languages = match client.fetch_languages(engine_for_langs) {
            Ok(l) => l,
            Err(e) => {
                self.translate_status = format!("Failed to load languages: {e}");
                return;
            }
        };

        self.loading_engines = false;
        self.loading_languages = false;
        self.translate_status.clear();

        if let Some(ref last_target) = self.settings.last_target_lang {
            self.selected_target_idx = self
                .languages
                .iter()
                .position(|l| &l.language_code == last_target)
                .unwrap_or(0);
        }

        self.prev_engine_idx = self.selected_engine_idx;
    }

    pub fn fetch_languages_for_engine(&mut self) {
        let auth_token = self.settings.auth_token.clone().unwrap_or_default();
        let client = ApiClient::new(&auth_token);

        let engine_name = self.engines.get(self.selected_engine_idx).map(|s| s.as_str());
        match client.fetch_languages(engine_name) {
            Ok(l) => {
                self.languages = l;
                self.selected_source_idx = 0;
                self.selected_target_idx = 0;
                self.prev_engine_idx = self.selected_engine_idx;
                self.loading_languages = false;
                self.translate_status.clear();
            }
            Err(e) => {
                self.translate_status = format!("Failed to load languages: {e}");
            }
        }
    }

    pub fn start_translation(&mut self, ctx: egui::Context) {
        let request = match &self.se_request {
            Some(r) => r.clone(),
            None => {
                self.translation_state = super::TranslationState::Error;
                self.translate_status = "No SE request loaded".to_string();
                return;
            }
        };

        let srt_content = match &request.subtitle.sub_rip {
            Some(s) => s.clone(),
            None => match &request.subtitle.native {
                Some(s) => s.clone(),
                None => {
                    self.translation_state = super::TranslationState::Error;
                    self.translate_status = "No subtitle content in request".to_string();
                    return;
                }
            },
        };

        let source_lang = self
            .languages
            .get(self.selected_source_idx)
            .map(|l| l.language_code.clone())
            .unwrap_or_else(|| "auto".to_string());

        let target_lang = self
            .languages
            .get(self.selected_target_idx)
            .map(|l| l.language_code.clone())
            .unwrap_or_else(|| "en".to_string());

        let engine = self
            .engines
            .get(self.selected_engine_idx)
            .cloned()
            .unwrap_or_default();

        let auth_token = self.settings.auth_token.clone().unwrap_or_default();

        self.translation_state = super::TranslationState::Translating;
        *self.progress.lock().unwrap() = 0.0;
        self.cancel_flag.store(false, Ordering::Relaxed);
        self.translate_status = "Translating...".to_string();
        *self.thread_result.lock().unwrap() = None;

        self.settings.last_source_lang = Some(source_lang.clone());
        self.settings.last_target_lang = Some(target_lang.clone());
        self.settings.last_engine = Some(engine.clone());
        self.save_settings_now();

        let cancel = self.cancel_flag.clone();
        let progress = self.progress.clone();
        let thread_result = self.thread_result.clone();

        std::thread::spawn(move || {
            let client = ApiClient::new(&auth_token);
            let result = client.translate(
                &srt_content,
                &source_lang,
                &target_lang,
                &engine,
                &cancel,
                &|p| {
                    if let Ok(mut prog) = progress.lock() {
                        *prog = p;
                    }
                },
            );

            let tr = match result {
                Ok(translated) => ThreadResult {
                    translated: Some(translated),
                    error: None,
                },
                Err(e) => ThreadResult {
                    translated: None,
                    error: Some(e.to_string()),
                },
            };

            if let Ok(mut res) = thread_result.lock() {
                *res = Some(tr);
            }

            ctx.request_repaint();
        });
    }

    pub fn check_thread_result(&mut self) {
        if let Ok(mut res) = self.thread_result.lock() {
            if let Some(tr) = res.take() {
                if let Some(translated) = tr.translated {
                    self.translated_srt = Some(translated);
                    self.translation_state = super::TranslationState::Done;
                    self.translate_status.clear();
                } else if let Some(err) = tr.error {
                    self.translation_state = super::TranslationState::Error;
                    self.translate_status = err;
                }
            }
        }
    }

    pub fn start_detect(&mut self, ctx: egui::Context) {
        let request = match &self.se_request {
            Some(r) => r.clone(),
            None => {
                self.translate_status = "No subtitle loaded".to_string();
                return;
            }
        };

        let srt_content = match (&request.subtitle.sub_rip, &request.subtitle.native) {
            (Some(s), _) | (_, Some(s)) => s.clone(),
            _ => {
                self.translate_status = "No subtitle content".to_string();
                return;
            }
        };

        let auth_token = self.settings.auth_token.clone().unwrap_or_default();
        let detect_result = self.detect_result.clone();

        self.detecting_language = true;
        self.translation_state = super::TranslationState::Detecting;
        self.translate_status = "Detecting language...".to_string();
        self.detect_start_time = Some(std::time::Instant::now());
        *detect_result.lock().unwrap() = None;

        std::thread::spawn(move || {
            let client = ApiClient::new(&auth_token);
            let result = match client.detect_language(&srt_content) {
                Ok(detected) => Ok(DetectResult {
                    iso_code: detected.iso_639_1.clone(),
                    w3c_code: detected.w3c.clone(),
                    language_name: detected.name.clone(),
                }),
                Err(e) => Err(e.to_string()),
            };

            if let Ok(mut res) = detect_result.lock() {
                *res = Some(result);
            }

            ctx.request_repaint();
        });
    }

    pub fn check_detect_result(&mut self) {
        if let Some(start) = self.detect_start_time {
            if start.elapsed() < std::time::Duration::from_millis(1500) {
                return;
            }
        }

        let result = if let Ok(mut res) = self.detect_result.lock() {
            res.take()
        } else {
            None
        };

        if let Some(result) = result {
            match result {
                Ok(detected) => {
                    if let Some(idx) = find_nearest_language_idx(&self.languages, &detected.w3c_code)
                        .or_else(|| find_nearest_language_idx(&self.languages, &detected.iso_code))
                    {
                        self.selected_source_idx = idx;
                        let msg = format!(
                            "Detected: {}",
                            self.languages.get(idx)
                                .map(|l| format!("{} ({})", l.language_name, l.language_code))
                                .unwrap_or_else(|| detected.language_name.clone())
                        );
                        self.toast(msg, egui::Color32::from_rgb(100, 200, 100));
                    } else {
                        let msg = format!(
                            "Detected: {} ({}) — no matching language in current engine",
                            detected.language_name, detected.w3c_code
                        );
                        self.toast(msg, egui::Color32::from_rgb(255, 180, 60));
                    }
                }
                Err(e) => {
                    self.toast(format!("Detection failed: {e}"), egui::Color32::from_rgb(220, 80, 80));
                }
            }
            self.detecting_language = false;
            self.detect_start_time = None;
            self.translation_state = super::TranslationState::Idle;
        }
    }

    pub fn draw_translate_tab(&mut self, ui: &mut egui::Ui, ctx: egui::Context) {
        match &self.translation_state {
            super::TranslationState::Idle | super::TranslationState::Detecting => {
                self.draw_translation_controls(ui, ctx);
            }
            super::TranslationState::Translating => {
                self.draw_translation_controls(ui, ctx.clone());
                ui.add_space(8.0);
                self.draw_progress(ui);
            }
            super::TranslationState::Done => {
                ui.colored_label(egui::Color32::GREEN, "Translation complete!");
                ui.add_space(8.0);
                if ui.button("OK").clicked() {
                    self.write_result();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            super::TranslationState::Error => {
                if ui.button("Close").clicked() {
                    self.write_result();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    fn draw_translation_controls(&mut self, ui: &mut egui::Ui, ctx: egui::Context) {
        if self.engines.is_empty() {
            ui.label("No translation engines available.");
            return;
        }

        ui.group(|ui| {
            ui.heading("Translation Settings");
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label("Engine:");
                let selected_text = self.engines.get(self.selected_engine_idx).map(|s| s.as_str()).unwrap_or("");
                egui::ComboBox::from_id_salt("engine_selector")
                    .selected_text(selected_text)
                    .show_ui(ui, |ui| {
                        for (i, name) in self.engines.iter().enumerate() {
                            ui.selectable_value(&mut self.selected_engine_idx, i, name);
                        }
                    });
            });

            ui.add_space(4.0);

            let lang_labels: Vec<String> = self
                .languages
                .iter()
                .map(|l| format!("{} ({})", l.language_name, l.language_code))
                .collect();

            ui.horizontal(|ui| {
                ui.label("Source:");
                let selected_source = lang_labels.get(self.selected_source_idx).map(|s| s.as_str()).unwrap_or("Select language");
                egui::ComboBox::from_id_salt("source_selector")
                    .selected_text(selected_source)
                    .show_ui(ui, |ui| {
                        for (i, name) in lang_labels.iter().enumerate() {
                            ui.selectable_value(&mut self.selected_source_idx, i, name);
                        }
                    });
                if ui.button("Detect").clicked() {
                    self.start_detect(ctx.clone());
                }
            });

            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label("Target:");
                let selected_target = lang_labels.get(self.selected_target_idx).map(|s| s.as_str()).unwrap_or("Select language");
                egui::ComboBox::from_id_salt("target_selector")
                    .selected_text(selected_target)
                    .show_ui(ui, |ui| {
                        for (i, name) in lang_labels.iter().enumerate() {
                            ui.selectable_value(&mut self.selected_target_idx, i, name);
                        }
                    });
            });

            ui.add_space(8.0);

            if ui.button("Translate").clicked() {
                self.start_translation(ctx.clone());
            }
        });
    }

    fn draw_progress(&mut self, ui: &mut egui::Ui) {
        let current_progress = *self.progress.lock().unwrap_or_else(|e| e.into_inner());

        ui.add(
            egui::ProgressBar::new(current_progress)
                .show_percentage()
                .animate(true),
        );

        ui.add_space(4.0);
        if ui.button("Cancel").clicked() {
            self.cancel_flag.store(true, Ordering::Relaxed);
            self.translation_state = super::TranslationState::Error;
            self.translate_status = "Translation cancelled.".to_string();
        }
    }

    pub fn draw_detect_dialog(&mut self, ctx: &egui::Context) {
        super::update::paint_backdrop(ctx);

        let mut do_cancel = false;

        egui::Window::new("Detecting Language")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(
                egui::Frame::window(&super::update::ui_style_for_modal(ctx))
                    .corner_radius(10.0)
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 4],
                        blur: 24,
                        spread: 4,
                        color: egui::Color32::from_black_alpha(120),
                    }),
            )
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.vertical_centered(|ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            egui::RichText::new("Analyzing subtitle content...")
                                .color(egui::Color32::from_gray(200)),
                        );
                    });
                    ui.add_space(14.0);
                });

                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        do_cancel = true;
                    }
                });
            });

        if do_cancel {
            self.detecting_language = false;
            self.detect_start_time = None;
            self.translation_state = super::TranslationState::Idle;
            self.toast("Language detection cancelled.", egui::Color32::from_gray(180));
        }
    }
}
