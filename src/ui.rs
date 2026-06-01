use crate::api::{ApiClient, LanguageInfo};
use crate::se_contract::{PluginSettings, SeRequest, SeResponse};
use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

enum AppState {
    Setup,
    Ready,
    Translating,
    Detecting,
    Done,
    Error,
}

struct ThreadResult {
    translated: Option<String>,
    error: Option<String>,
}

struct DetectResult {
    iso_code: String,
    w3c_code: String,
    language_name: String,
}

fn find_nearest_language_idx(languages: &[LanguageInfo], detected_code: &str) -> Option<usize> {
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

pub struct TranslatorApp {
    settings: PluginSettings,
    se_request: Option<SeRequest>,
    response_path: Option<String>,
    state: AppState,
    status_message: String,

    engines: Vec<String>,
    languages: Vec<LanguageInfo>,
    selected_engine_idx: usize,
    prev_engine_idx: usize,
    selected_source_idx: usize,
    selected_target_idx: usize,

    login_username: String,
    login_password: String,

    cancel_flag: Arc<AtomicBool>,
    progress: Arc<Mutex<f32>>,
    thread_result: Arc<Mutex<Option<ThreadResult>>>,
    translated_srt: Option<String>,

    loading_engines: bool,
    loading_languages: bool,
    came_from_ready: bool,
    detecting_language: bool,
    detect_result: Arc<Mutex<Option<Result<DetectResult, String>>>>,
}

impl TranslatorApp {
    pub fn new(cc: &eframe::CreationContext<'_>, settings: PluginSettings) -> Self {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "my_font".to_owned(),
            Arc::new(egui::FontData::from_static(include_bytes!(
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"
            ))),
        );
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "my_font".to_owned());

        cc.egui_ctx.set_fonts(fonts);

        Self {
            settings,
            se_request: None,
            response_path: None,
            state: AppState::Setup,
            status_message: String::new(),
            engines: Vec::new(),
            languages: Vec::new(),
            selected_engine_idx: 0,
            prev_engine_idx: 0,
            selected_source_idx: 0,
            selected_target_idx: 0,
            login_username: String::new(),
            login_password: String::new(),
            cancel_flag: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(Mutex::new(0.0)),
            thread_result: Arc::new(Mutex::new(None)),
            translated_srt: None,
            loading_engines: false,
            loading_languages: false,
            came_from_ready: false,
            detecting_language: false,
            detect_result: Arc::new(Mutex::new(None)),
        }
    }

    pub fn set_request(&mut self, request: SeRequest) {
        self.response_path = Some(request.response_file_path.clone());

        let loaded_settings = PluginSettings::from_se_settings(request.settings.as_ref());
        if loaded_settings.has_credentials() {
            self.settings = loaded_settings;
            self.state = AppState::Ready;
            self.status_message = "Loading engines and languages...".to_string();
            self.loading_engines = true;
        } else {
            self.settings = loaded_settings;
            self.state = AppState::Setup;
            self.status_message = "Please log in to continue.".to_string();
        }

        self.se_request = Some(request);
    }

    fn fetch_engines_and_languages(&mut self) {
        let auth_token = self.settings.auth_token.clone().unwrap_or_default();

        let client = ApiClient::new(&auth_token);
        self.engines = match client.fetch_engines() {
            Ok(e) => e,
            Err(e) => {
                self.status_message = format!("Failed to load engines: {e}");
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
                self.status_message = format!("Failed to load languages: {e}");
                return;
            }
        };

        self.loading_engines = false;
        self.loading_languages = false;
        self.status_message.clear();

        if let Some(ref last_target) = self.settings.last_target_lang {
            self.selected_target_idx = self
                .languages
                .iter()
                .position(|l| &l.language_code == last_target)
                .unwrap_or(0);
        }

        self.prev_engine_idx = self.selected_engine_idx;
    }

    fn fetch_languages_for_engine(&mut self) {
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
                self.status_message.clear();
            }
            Err(e) => {
                self.status_message = format!("Failed to load languages: {e}");
            }
        }
    }

    fn start_translation(&mut self, ctx: egui::Context) {
        let request = match &self.se_request {
            Some(r) => r.clone(),
            None => {
                self.state = AppState::Error;
                self.status_message = "No SE request loaded".to_string();
                return;
            }
        };

        let srt_content = match &request.subtitle.sub_rip {
            Some(s) => s.clone(),
            None => match &request.subtitle.native {
                Some(s) => s.clone(),
                None => {
                    self.state = AppState::Error;
                    self.status_message = "No subtitle content in request".to_string();
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

        self.state = AppState::Translating;
        *self.progress.lock().unwrap() = 0.0;
        self.cancel_flag.store(false, Ordering::Relaxed);
        self.status_message = "Translating...".to_string();
        *self.thread_result.lock().unwrap() = None;

        self.settings.last_source_lang = Some(source_lang.clone());
        self.settings.last_target_lang = Some(target_lang.clone());
        self.settings.last_engine = Some(engine.clone());

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

    fn do_login(&mut self) {
        if self.login_username.is_empty() || self.login_password.is_empty() {
            self.status_message = "Username and password are required.".to_string();
            return;
        }

        self.status_message = "Logging in...".to_string();

        let username = self.login_username.clone();
        let password = self.login_password.clone();

        match ApiClient::login(&username, &password) {
            Ok(token) => {
                self.settings.auth_token = Some(token);
                self.login_password.clear();
                self.state = AppState::Ready;
                self.status_message = "Logged in. Loading engines and languages...".to_string();
                self.loading_engines = true;
            }
            Err(e) => {
                self.status_message = format!("Login failed: {e}");
            }
        }
    }

    fn write_result(&self) {
        if let (Some(path), Some(response)) = (&self.response_path, self.build_response()) {
            let _ = crate::se_contract::write_response(&response, path);
        }
    }

    fn build_response(&self) -> Option<SeResponse> {
        match &self.state {
            AppState::Done => {
                let translated = self.translated_srt.as_deref().unwrap_or("");
                Some(SeResponse::ok(translated, &self.settings))
            }
            AppState::Error => {
                Some(SeResponse::error(&self.status_message, &self.settings))
            }
            _ => Some(SeResponse::cancelled(&self.settings)),
        }
    }

    fn check_thread_result(&mut self) {
        if let Ok(mut res) = self.thread_result.lock() {
            if let Some(tr) = res.take() {
                if let Some(translated) = tr.translated {
                    self.translated_srt = Some(translated);
                    self.state = AppState::Done;
                    self.status_message.clear();
                } else if let Some(err) = tr.error {
                    self.state = AppState::Error;
                    self.status_message = err;
                }
            }
        }
    }

    fn start_detect(&mut self, ctx: egui::Context) {
        let request = match &self.se_request {
            Some(r) => r.clone(),
            None => {
                self.status_message = "No subtitle loaded".to_string();
                return;
            }
        };

        let srt_content = match (&request.subtitle.sub_rip, &request.subtitle.native) {
            (Some(s), _) | (_, Some(s)) => s.clone(),
            _ => {
                self.status_message = "No subtitle content".to_string();
                return;
            }
        };

        let auth_token = self.settings.auth_token.clone().unwrap_or_default();
        let detect_result = self.detect_result.clone();

        self.detecting_language = true;
        self.state = AppState::Detecting;
        self.status_message = "Detecting language...".to_string();
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

    fn check_detect_result(&mut self) {
        if let Ok(mut res) = self.detect_result.lock() {
            if let Some(result) = res.take() {
                match result {
                    Ok(detected) => {
                        if let Some(idx) = find_nearest_language_idx(&self.languages, &detected.w3c_code)
                            .or_else(|| find_nearest_language_idx(&self.languages, &detected.iso_code))
                        {
                            self.selected_source_idx = idx;
                            self.status_message = format!(
                                "Detected: {}",
                                self.languages.get(idx)
                                    .map(|l| format!("{} ({})", l.language_name, l.language_code))
                                    .unwrap_or_else(|| detected.language_name.clone())
                            );
                        } else {
                            self.status_message = format!(
                                "Detected: {} ({}) — no matching language in current engine",
                                detected.language_name, detected.w3c_code
                            );
                        }
                    }
                    Err(e) => {
                        self.status_message = format!("Detection failed: {e}");
                    }
                }
                self.detecting_language = false;
                self.state = AppState::Ready;
            }
        }
    }
}

impl eframe::App for TranslatorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.loading_engines {
            self.fetch_engines_and_languages();
            ctx.request_repaint();
        }

        if self.loading_languages {
            self.fetch_languages_for_engine();
            ctx.request_repaint();
        }

        if !self.loading_engines && !self.loading_languages && self.selected_engine_idx != self.prev_engine_idx {
            self.loading_languages = true;
            self.status_message = "Loading languages...".to_string();
            ctx.request_repaint();
        }

        if matches!(self.state, AppState::Translating) {
            self.check_thread_result();
            ctx.request_repaint();
        }

        if matches!(self.state, AppState::Detecting) {
            self.check_detect_result();
            ctx.request_repaint();
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(8.0);
                ui.heading("AI Translate (OpenSubtitles)");
                ui.add_space(4.0);

                if !self.status_message.is_empty() && !matches!(self.state, AppState::Translating) {
                    ui.colored_label(egui::Color32::from_rgb(255, 200, 100), &self.status_message);
                    ui.add_space(4.0);
                }

                match self.state {
                    AppState::Setup => {
                        self.draw_setup(ui);
                    }
                    AppState::Ready => {
                        self.draw_translation(ui, ctx.clone());
                    }
                    AppState::Detecting => {
                        ui.spinner();
                        ui.label(&self.status_message);
                    }
                    AppState::Translating => {
                        self.draw_progress(ui);
                    }
                    AppState::Done => {
                        ui.colored_label(egui::Color32::GREEN, "Translation complete!");
                        if ui.button("OK").clicked() {
                            self.write_result();
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                    AppState::Error => {
                        ui.colored_label(egui::Color32::RED, &self.status_message);
                        ui.add_space(8.0);
                        if ui.button("Close").clicked() {
                            self.write_result();
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                }
            });
        });

        self.write_result_on_close(ctx);
    }
}

impl TranslatorApp {
    fn write_result_on_close(&self, ctx: &egui::Context) {
        let is_closing = ctx.input(|i| i.viewport().close_requested());
        if is_closing && !matches!(self.state, AppState::Done) {
            self.write_result();
        }
    }

    fn draw_setup(&mut self, ui: &mut egui::Ui) {
        if self.came_from_ready {
            if ui.button("← Back").clicked() {
                self.state = AppState::Ready;
                self.status_message.clear();
            }
            ui.add_space(8.0);
        }

        ui.group(|ui| {
            ui.heading("Login");
            ui.add(
                egui::TextEdit::singleline(&mut self.login_username)
                    .hint_text("Username")
                    .desired_width(300.0),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.login_password)
                    .hint_text("Password")
                    .password(true)
                    .desired_width(300.0),
            );
            ui.add_space(4.0);
            if ui.button("Login").clicked() {
                self.do_login();
            }
        });
    }

    fn draw_translation(&mut self, ui: &mut egui::Ui, ctx: egui::Context) {
        if self.engines.is_empty() {
            ui.label("No translation engines available.");
            return;
        }

        ui.group(|ui| {
            ui.heading("Translation Settings");
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label("Engine:");
                let default_engine = "";
                let selected_text = self.engines.get(self.selected_engine_idx).map(|s| s.as_str()).unwrap_or(default_engine);
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
                let default_source = "Select language";
                let selected_source = lang_labels.get(self.selected_source_idx).map(|s| s.as_str()).unwrap_or(default_source);
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
                let default_target = "Select language";
                let selected_target = lang_labels.get(self.selected_target_idx).map(|s| s.as_str()).unwrap_or(default_target);
                egui::ComboBox::from_id_salt("target_selector")
                    .selected_text(selected_target)
                    .show_ui(ui, |ui| {
                        for (i, name) in lang_labels.iter().enumerate() {
                            ui.selectable_value(&mut self.selected_target_idx, i, name);
                        }
                    });
            });

            ui.add_space(8.0);

            ui.horizontal(|ui| {
                if ui.button("Translate").clicked() {
                    self.start_translation(ctx.clone());
                }
                if ui.button("Settings").clicked() {
                    self.came_from_ready = true;
                    self.state = AppState::Setup;
                }
            });
        });
    }

    fn draw_progress(&mut self, ui: &mut egui::Ui) {
        let current_progress = *self.progress.lock().unwrap_or_else(|e| e.into_inner());

        ui.add_space(16.0);
        ui.label(&self.status_message);
        ui.add_space(8.0);
        ui.add(
            egui::ProgressBar::new(current_progress)
                .show_percentage()
                .animate(true),
        );

        ui.add_space(8.0);
        if ui.button("Cancel").clicked() {
            self.cancel_flag.store(true, Ordering::Relaxed);
            self.state = AppState::Error;
            self.status_message = "Translation cancelled.".to_string();
        }
    }
}
