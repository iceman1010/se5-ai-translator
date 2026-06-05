use crate::api::{ApiClient, LanguageInfo, TranslationUsage};
use crate::debug_log;
use eframe::egui;
use std::sync::atomic::Ordering;

use super::TranslatorApp;

pub struct ThreadResult {
    pub translated: Option<String>,
    pub usage: Option<TranslationUsage>,
    /// If the worker thread refreshed its auth token mid-flight, the new
    /// token lands here so the UI can persist it to PluginSettings.
    pub refreshed_token: Option<String>,
    pub error: Option<String>,
}

pub struct DetectResult {
    pub iso_code: String,
    pub w3c_code: String,
    pub language_name: String,
}

/// Wrapper carrying the detect worker's outcome plus any side-effects
/// produced mid-flight (refreshed token) so the UI thread can persist them.
pub struct DetectThreadResult {
    pub result: Result<DetectResult, String>,
    pub refreshed_token: Option<String>,
}

pub fn find_nearest_language_idx(languages: &[LanguageInfo], detected_code: &str) -> Option<usize> {
    let detected_lower = detected_code.to_lowercase();

    languages
        .iter()
        .position(|l| l.language_code.to_lowercase() == detected_lower)
        .or_else(|| {
            let base_detected = detected_lower.split('-').next().unwrap_or(&detected_lower);
            let base_detected2 = detected_lower.split('_').next().unwrap_or(&detected_lower);
            languages.iter().position(|l| {
                let lc_lower = l.language_code.to_lowercase();
                let base_lang = lc_lower.split('-').next().unwrap_or("");
                let base_lang2 = lc_lower.split('_').next().unwrap_or("");
                base_lang == base_detected
                    || base_lang2 == base_detected2
                    || base_detected == base_lang
                    || base_detected2 == base_lang2
            })
        })
}

impl TranslatorApp {
    pub fn fetch_engines_and_languages(&mut self) {
        let auth_token = self.settings.auth_token.clone().unwrap_or_default();
        let username = self.settings.username.clone();
        let password = self.settings.password.clone();

        let mut client = ApiClient::with_credentials(&auth_token, username, password, None);
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

        let engine_for_langs = self
            .engines
            .get(self.selected_engine_idx)
            .map(|s| s.as_str());
        self.languages = match client.fetch_languages(engine_for_langs) {
            Ok(l) => l,
            Err(e) => {
                self.translate_status = format!("Failed to load languages: {e}");
                return;
            }
        };

        self.persist_refreshed_token(&mut client);
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
        let username = self.settings.username.clone();
        let password = self.settings.password.clone();
        let mut client = ApiClient::with_credentials(&auth_token, username, password, None);

        let engine_name = self
            .engines
            .get(self.selected_engine_idx)
            .map(|s| s.as_str());
        match client.fetch_languages(engine_name) {
            Ok(l) => {
                self.languages = l;
                self.selected_source_idx = 0;
                self.selected_target_idx = 0;
                self.prev_engine_idx = self.selected_engine_idx;
                self.loading_languages = false;
                self.translate_status.clear();
                self.persist_refreshed_token(&mut client);
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
        let username = self.settings.username.clone();
        let password = self.settings.password.clone();

        self.translation_state = super::TranslationState::Translating;
        *self.progress.lock().unwrap() = 0.0;
        self.cancel_flag.store(false, Ordering::Relaxed);
        self.translate_status = "Translating...".to_string();
        *self.thread_result.lock().unwrap() = None;
        self.translation_start = Some(std::time::Instant::now());
        self.done_at = None;
        self.hold_until = None;
        self.last_usage = None;
        *self.auth_status.lock().unwrap() = None;

        self.settings.last_source_lang = Some(source_lang.clone());
        self.settings.last_target_lang = Some(target_lang.clone());
        self.settings.last_engine = Some(engine.clone());
        self.save_settings_now();

        let cancel = self.cancel_flag.clone();
        let progress = self.progress.clone();
        let thread_result = self.thread_result.clone();
        let auth_status = self.auth_status.clone();

        std::thread::spawn(move || {
            let mut client =
                ApiClient::with_credentials(&auth_token, username, password, Some(auth_status));
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

            let refreshed_token = client.take_refreshed_token();

            let tr = match result {
                Ok(res) => ThreadResult {
                    translated: Some(res.translation),
                    usage: res.usage,
                    refreshed_token,
                    error: None,
                },
                Err(e) => ThreadResult {
                    translated: None,
                    usage: None,
                    refreshed_token,
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
        let result = if let Ok(mut res) = self.thread_result.lock() {
            res.take()
        } else {
            None
        };

        if let Some(tr) = result {
            // Always persist a refreshed token, regardless of success/failure
            // — the new token is valid even if the original call failed.
            if let Some(new_token) = tr.refreshed_token {
                debug_log!("check_thread_result: saving refreshed auth token");
                self.settings.auth_token = Some(new_token);
                self.save_settings_now();
            }
            // Clear any lingering "Re-authenticating…" status message.
            *self.auth_status.lock().unwrap() = None;

            if let Some(translated) = tr.translated {
                self.translated_srt = Some(translated);

                // Update balance + carry usage info into the UI.
                if let Some(u) = &tr.usage {
                    self.credits_balance = Some(u.credits_left);
                    self.last_usage = Some(u.clone());
                }

                // Compute hold-until timestamp.
                // The completion modal must be visible for at least MIN_HOLD
                // after the translation finishes so the user has time to
                // read cost / remaining credits. If the translation was
                // faster than MIN_HOLD, hold for the unused portion;
                // otherwise hold for a fresh MIN_HOLD window.
                const MIN_HOLD: std::time::Duration = std::time::Duration::from_secs(6);

                let now = std::time::Instant::now();
                let done_at = now;
                let hold_until = match self.translation_start {
                    Some(start) if now.duration_since(start) < MIN_HOLD => {
                        // Hold for the unused portion of MIN_HOLD.
                        let remaining = MIN_HOLD - now.duration_since(start);
                        now + remaining
                    }
                    _ => now + MIN_HOLD,
                };
                self.done_at = Some(done_at);
                self.hold_until = Some(hold_until);

                self.translation_state = super::TranslationState::Done;
            } else if let Some(err) = tr.error {
                self.translation_state = super::TranslationState::Error;
                self.toast(err, egui::Color32::from_rgb(220, 80, 80));
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
        let username = self.settings.username.clone();
        let password = self.settings.password.clone();
        let detect_result = self.detect_result.clone();
        let auth_status = self.auth_status.clone();

        self.detecting_language = true;
        self.translation_state = super::TranslationState::Detecting;
        self.translate_status = "Detecting language...".to_string();
        self.detect_start_time = Some(std::time::Instant::now());
        *detect_result.lock().unwrap() = None;
        *self.auth_status.lock().unwrap() = None;

        std::thread::spawn(move || {
            let mut client =
                ApiClient::with_credentials(&auth_token, username, password, Some(auth_status));
            let result = match client.detect_language(&srt_content) {
                Ok(detected) => Ok(DetectResult {
                    iso_code: detected.iso_639_1.clone(),
                    w3c_code: detected.w3c.clone(),
                    language_name: detected.name.clone(),
                }),
                Err(e) => Err(e.to_string()),
            };
            // Always drain the refreshed token (if any) so it can be
            // persisted on the UI thread — even when detect itself failed,
            // the new token is valid and shouldn't be discarded.
            let refreshed_token = client.take_refreshed_token();

            let tr = DetectThreadResult {
                result,
                refreshed_token,
            };

            if let Ok(mut res) = detect_result.lock() {
                *res = Some(tr);
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

        if let Some(tr) = result {
            // Persist any refreshed token before doing anything else — the
            // new token is valid even if detect itself failed.
            if let Some(new_token) = tr.refreshed_token {
                debug_log!("check_detect_result: saving refreshed auth token");
                self.settings.auth_token = Some(new_token);
                self.save_settings_now();
            }
            // Clear any lingering "Re-authenticating…" status message.
            *self.auth_status.lock().unwrap() = None;

            match tr.result {
                Ok(detected) => {
                    if let Some(idx) =
                        find_nearest_language_idx(&self.languages, &detected.w3c_code).or_else(
                            || find_nearest_language_idx(&self.languages, &detected.iso_code),
                        )
                    {
                        self.selected_source_idx = idx;
                        let msg = format!(
                            "Detected: {}",
                            self.languages
                                .get(idx)
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
                    self.toast(
                        format!("Detection failed: {e}"),
                        egui::Color32::from_rgb(220, 80, 80),
                    );
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
                // Controls stay visible behind the modal backdrop; the
                // completion modal itself is rendered from `update()` so
                // it overlays the entire window.
                self.draw_translation_controls(ui, ctx.clone());
            }
            super::TranslationState::Error => {
                ui.vertical_centered(|ui| {
                    ui.add_space(12.0);
                    ui.colored_label(
                        egui::Color32::from_rgb(220, 80, 80),
                        "Translation failed — see notification.",
                    );
                    ui.add_space(8.0);
                    if ui.button("Close").clicked() {
                        self.write_result();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            }
        }
    }

    fn draw_translation_controls(&mut self, ui: &mut egui::Ui, ctx: egui::Context) {
        if self.engines.is_empty() {
            ui.label("No translation engines available.");
            return;
        }

        let lang_labels: Vec<String> = self
            .languages
            .iter()
            .map(|l| format!("{} ({})", l.language_name, l.language_code))
            .collect();

        let combo_width = 220.0;

        ui.group(|ui| {
            ui.add_space(6.0);
            ui.heading("Translation Settings");
            ui.add_space(6.0);

            egui::Grid::new("translation_settings_grid")
                .num_columns(3)
                .spacing([12.0, 8.0])
                .striped(true)
                .min_col_width(70.0)
                .show(ui, |ui| {
                    // Row: Engine
                    ui.label(egui::RichText::new("Engine").strong());
                    ui.set_min_width(combo_width);
                    let selected_text = self
                        .engines
                        .get(self.selected_engine_idx)
                        .map(|s| s.as_str())
                        .unwrap_or("");
                    egui::ComboBox::from_id_salt("engine_selector")
                        .selected_text(selected_text)
                        .show_ui(ui, |ui| {
                            for (i, name) in self.engines.iter().enumerate() {
                                ui.selectable_value(&mut self.selected_engine_idx, i, name);
                            }
                        });
                    ui.label("");
                    ui.end_row();

                    // Row: Source
                    ui.label(egui::RichText::new("Source").strong());
                    ui.set_min_width(combo_width);
                    let selected_source = lang_labels
                        .get(self.selected_source_idx)
                        .map(|s| s.as_str())
                        .unwrap_or("Select language");
                    egui::ComboBox::from_id_salt("source_selector")
                        .selected_text(selected_source)
                        .show_ui(ui, |ui| {
                            for (i, name) in lang_labels.iter().enumerate() {
                                ui.selectable_value(&mut self.selected_source_idx, i, name);
                            }
                        });
                    ui.vertical(|ui| {
                        ui.add_space(2.0);
                        if ui.button("Detect").clicked() {
                            self.start_detect(ctx.clone());
                        }
                    });
                    ui.end_row();

                    // Row: Target
                    ui.label(egui::RichText::new("Target").strong());
                    ui.set_min_width(combo_width);
                    let selected_target = lang_labels
                        .get(self.selected_target_idx)
                        .map(|s| s.as_str())
                        .unwrap_or("Select language");
                    egui::ComboBox::from_id_salt("target_selector")
                        .selected_text(selected_target)
                        .show_ui(ui, |ui| {
                            for (i, name) in lang_labels.iter().enumerate() {
                                ui.selectable_value(&mut self.selected_target_idx, i, name);
                            }
                        });
                    ui.label("");
                    ui.end_row();
                });

            ui.add_space(10.0);
            ui.vertical_centered(|ui| {
                let btn = egui::Button::new(
                    egui::RichText::new("Translate")
                        .color(egui::Color32::WHITE)
                        .size(14.0),
                )
                .min_size(egui::vec2(140.0, 0.0))
                .corner_radius(6.0)
                .fill(egui::Color32::from_rgb(90, 142, 242));
                if ui.add(btn).clicked() {
                    self.start_translation(ctx.clone());
                }
            });
            ui.add_space(6.0);
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

        // Surface transient API client messages (e.g. "Session expired,
        // re-authenticating…") right under the progress bar so the user has
        // context for a longer-than-expected translation.
        let status_msg = self
            .auth_status
            .lock()
            .ok()
            .and_then(|g| g.clone());
        if let Some(msg) = status_msg {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.colored_label(
                    egui::Color32::from_rgb(255, 200, 100),
                    egui::RichText::new(msg).strong(),
                );
            });
            ui.add_space(2.0);
        }

        if ui.button("Cancel").clicked() {
            self.cancel_flag.store(true, Ordering::Relaxed);
            self.translation_state = super::TranslationState::Error;
            self.translate_status = "Translation cancelled.".to_string();
        }
    }

    /// Renders the post-translation summary (cost + remaining credits + a
    /// countdown to auto-close) as a centered modal window with a backdrop.
    /// Called from `update()` while `translation_state == Done`.
    pub fn draw_completion_modal(&mut self, ctx: &egui::Context) {
        super::update::paint_backdrop(ctx);

        let now = std::time::Instant::now();
        let secs_left = self
            .hold_until
            .map(|t| {
                let d = t.saturating_duration_since(now);
                d.as_secs_f32().max(0.0)
            })
            .unwrap_or(0.0);

        egui::Window::new("Translation Complete")
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
                ui.add_space(6.0);
                ui.vertical_centered(|ui| {
                    ui.colored_label(
                        egui::Color32::from_rgb(100, 200, 100),
                        egui::RichText::new("✓ Translation complete!").strong().size(16.0),
                    );
                    ui.add_space(10.0);

                    if let Some(u) = &self.last_usage {
                        egui::Grid::new("usage_grid_modal")
                            .num_columns(2)
                            .spacing([14.0, 5.0])
                            .min_col_width(130.0)
                            .show(ui, |ui| {
                                ui.label(egui::RichText::new("Cost:").strong());
                                ui.label(format_credits(u.total_price));
                                ui.end_row();

                                ui.label(egui::RichText::new("Characters:").strong());
                                ui.label(format!("{}", u.characters_count));
                                ui.end_row();

                                ui.label(egui::RichText::new("Remaining:").strong());
                                ui.label(format_credits(u.credits_left));
                                ui.end_row();

                                if u.duration > 0.0 {
                                    ui.label(egui::RichText::new("Duration:").strong());
                                    ui.label(format!("{:.1}s", u.duration));
                                    ui.end_row();
                                }
                            });
                    } else {
                        ui.label(
                            egui::RichText::new("(no usage info returned by server)")
                                .italics()
                                .color(egui::Color32::GRAY),
                        );
                    }

                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new(format!("Applying in {secs_left:.0}s…"))
                            .small()
                            .color(egui::Color32::from_gray(170)),
                    );
                });
                ui.add_space(4.0);
            });
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
            self.toast(
                "Language detection cancelled.",
                egui::Color32::from_gray(180),
            );
        }
    }
}

/// Renders a credit value as an integer when whole, otherwise with 2 decimals.
///
/// The API returns credit balances as numbers that are practically always
/// integers (e.g. `9092`, `2`), but the JSON type is `f64` — so we strip the
/// trailing `.0` when the value is whole for a cleaner UI.
fn format_credits(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        format!("{value:.2}")
    }
}
