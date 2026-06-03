use crate::debug_log;
use crate::se_contract::{PluginSettings, SeRequest, SeResponse};
use eframe::egui;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::api::LanguageInfo;
use crate::ui::translate::{DetectResult, ThreadResult};
use crate::ui::update::{UpdateCheckResult, UpdateDownloadResult, UpdateState};

mod account;
mod ai_models;
mod credits;
mod translate;
mod update;

const ICON_PNG: &[u8] = include_bytes!("../../icons/icon.png");
const APP_DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");
const REPO_URL: &str = env!("CARGO_PKG_REPOSITORY");

#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    Translate,
    AiModels,
    Settings,
    Account,
    Credits,
}

pub enum TranslationState {
    Idle,
    Translating,
    Detecting,
    Done,
    Error,
}

pub struct TranslatorApp {
    settings: PluginSettings,
    se_request: Option<SeRequest>,
    response_path: Option<String>,
    pub translation_state: TranslationState,
    pub active_tab: Tab,
    pub translate_status: String,
    pub account_status: String,

    pub engines: Vec<String>,
    pub languages: Vec<LanguageInfo>,
    pub selected_engine_idx: usize,
    pub prev_engine_idx: usize,
    pub selected_source_idx: usize,
    pub selected_target_idx: usize,

    pub login_username: String,
    pub login_password: String,

    pub cancel_flag: Arc<AtomicBool>,
    pub progress: Arc<Mutex<f32>>,
    pub thread_result: Arc<Mutex<Option<ThreadResult>>>,
    pub translated_srt: Option<String>,

    pub first_frame: bool,
    pub loading_engines: bool,
    pub loading_languages: bool,
    pub detecting_language: bool,
    pub detect_result: Arc<Mutex<Option<Result<DetectResult, String>>>>,

    pub update_state: UpdateState,
    pub update_check_result: UpdateCheckResult,
    pub update_download_result: UpdateDownloadResult,
    pub show_update_dialog: bool,

    logo_texture: Option<egui::TextureHandle>,
}

impl TranslatorApp {
    pub fn new(cc: &eframe::CreationContext<'_>, settings: PluginSettings) -> Self {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "my_font".to_owned(),
            Arc::new(egui::FontData::from_static(include_bytes!(
                "../../assets/DejaVuSans.ttf"
            ))),
        );
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "my_font".to_owned());

        cc.egui_ctx.set_fonts(fonts);

        let logo_texture = {
            let image = image::load_from_memory(ICON_PNG)
                .expect("failed to load embedded icon.png");
            let size = [image.width() as usize, image.height() as usize];
            let pixels: Vec<egui::Color32> = image
                .to_rgba8()
                .pixels()
                .map(|p| egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
                .collect();
            let color_image = egui::ColorImage { size, pixels };
            Some(cc.egui_ctx.load_texture(
                "logo",
                color_image,
                egui::TextureOptions::LINEAR,
            ))
        };

        Self {
            settings,
            se_request: None,
            response_path: None,
            translation_state: TranslationState::Idle,
            active_tab: Tab::Translate,
            translate_status: String::new(),
            account_status: String::new(),
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
            first_frame: true,
            loading_engines: false,
            loading_languages: false,
            detecting_language: false,
            detect_result: Arc::new(Mutex::new(None)),
            update_state: UpdateState::Idle,
            update_check_result: Arc::new(Mutex::new(None)),
            update_download_result: Arc::new(Mutex::new(None)),
            show_update_dialog: false,
            logo_texture,
        }
    }

    pub fn set_request(&mut self, request: SeRequest) {
        debug_log!("set_request: response_path={} raw_settings={:?}", request.response_file_path, request.settings);
        self.response_path = Some(request.response_file_path.clone());

        let loaded_settings = PluginSettings::from_se_settings(request.settings.as_ref());
        debug_log!("loaded_settings: auth_token={:?} username={:?} password={:?} has_creds={}",
            loaded_settings.auth_token, loaded_settings.username, loaded_settings.password, loaded_settings.has_credentials());
        self.settings = loaded_settings;

        if let Some(ref u) = self.settings.username {
            self.login_username = u.clone();
        }

        if self.settings.has_credentials() {
            self.translation_state = TranslationState::Idle;
            self.active_tab = Tab::Translate;
            self.translate_status = "Loading engines and languages...".to_string();
            self.loading_engines = true;
        } else {
            self.translation_state = TranslationState::Idle;
            self.active_tab = Tab::Account;
            self.account_status = "Please log in to continue.".to_string();
        }

        self.se_request = Some(request);
    }

    pub fn write_result(&self) {
        if let (Some(path), Some(response)) = (&self.response_path, self.build_response()) {
            debug_log!("write_result: status={} settings={{auth_token={:?}, username={:?}}}",
                response.status, response.settings.as_ref().and_then(|s| s.get("authToken")),
                response.settings.as_ref().and_then(|s| s.get("username")));
            let _ = crate::se_contract::write_response(&response, path);
        }
    }

    pub fn save_settings_now(&self) {
        self.write_result();
    }

    fn build_response(&self) -> Option<SeResponse> {
        match &self.translation_state {
            TranslationState::Done => {
                let translated = self.translated_srt.as_deref().unwrap_or("");
                Some(SeResponse::ok(translated, &self.settings))
            }
            TranslationState::Error => {
                Some(SeResponse::error(&self.translate_status, &self.settings))
            }
            _ => Some(SeResponse::cancelled(&self.settings)),
        }
    }

    fn write_result_on_close(&self, ctx: &egui::Context) {
        let is_closing = ctx.input(|i| i.viewport().close_requested());
        if is_closing {
            self.write_result();
        }
    }

    fn draw_tab_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let tabs = [
                (Tab::Translate, "Translate"),
                (Tab::AiModels, "AI Models"),
                (Tab::Settings, "Settings"),
                (Tab::Account, "Account"),
                (Tab::Credits, "Credits"),
            ];
            for (tab, label) in tabs {
                if ui.selectable_label(self.active_tab == tab, label).clicked() {
                    self.active_tab = tab;
                }
            }
        });
        ui.separator();
    }
}

impl eframe::App for TranslatorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.first_frame {
            self.first_frame = false;
            self.write_result();
        }

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
            self.translate_status = "Loading languages...".to_string();
            ctx.request_repaint();
        }

        if matches!(self.translation_state, TranslationState::Translating) {
            self.check_thread_result();
            ctx.request_repaint();
        }

        if matches!(self.translation_state, TranslationState::Detecting) {
            self.check_detect_result();
            ctx.request_repaint();
        }

        if matches!(self.update_state, UpdateState::Checking) {
            self.process_update_result();
            ctx.request_repaint();
        }

        if matches!(self.update_state, UpdateState::Downloading { .. }) {
            self.process_download_result();
            ctx.request_repaint();
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(8.0);
                if let Some(tex) = &self.logo_texture {
                    let size = egui::Vec2::splat(48.0);
                    ui.image(egui::load::SizedTexture::new(tex.id(), size));
                }
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(APP_DESCRIPTION).small().color(egui::Color32::GRAY),
                );
                ui.add_space(4.0);

                if !self.settings.has_credentials() {
                    self.draw_login_screen(ui);
                } else {
                    self.draw_tab_bar(ui);
                    ui.add_space(4.0);
                    match self.active_tab {
                        Tab::Translate => self.draw_translate_tab(ui, ctx.clone()),
                        Tab::AiModels => self.draw_ai_models_tab(ui),
                        Tab::Settings => self.draw_settings_tab(ui, ctx.clone()),
                        Tab::Account => self.draw_account_tab(ui),
                        Tab::Credits => self.draw_credits_tab(ui),
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    ui.label(egui::RichText::new(concat!("v", env!("CARGO_PKG_VERSION"))).small().color(egui::Color32::GRAY));
                });
            });
        });

        if self.show_update_dialog {
            self.draw_update_dialog(ctx);
        }

        self.write_result_on_close(ctx);
    }
}

impl Drop for TranslatorApp {
    fn drop(&mut self) {
        self.write_result();
    }
}
