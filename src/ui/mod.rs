use crate::debug_log;
use crate::se_contract::{PluginSettings, SeRequest, SeResponse};
use eframe::egui;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::api::LanguageInfo;
use crate::api::ServiceModel;
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

#[derive(Clone, Copy, PartialEq)]
pub enum CreditsLoadState {
    Idle,
    Loading,
    Loaded,
    Error,
}

pub struct Toast {
    pub message: String,
    pub color: egui::Color32,
    pub created_at: std::time::Instant,
}

const TOAST_DURATION: std::time::Duration = std::time::Duration::from_secs(4);
const TOAST_FADE: std::time::Duration = std::time::Duration::from_millis(500);

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
    pub detect_start_time: Option<std::time::Instant>,

    pub update_state: UpdateState,
    pub update_check_result: UpdateCheckResult,
    pub update_download_result: UpdateDownloadResult,
    pub update_progress: Arc<Mutex<f32>>,
    pub show_update_dialog: bool,

    pub credits_balance: Option<f64>,
    pub credits_packages: Vec<crate::api::CreditPackage>,
    pub credits_state: CreditsLoadState,
    pub credits_error: String,

    pub services_info: Vec<ServiceModel>,
    pub services_state: CreditsLoadState,
    pub services_error: String,
    pub selected_service_idx: Option<usize>,
    /// Which model's language list is currently shown in the modal (None = closed).
    pub services_langs_modal_idx: Option<usize>,
    /// Current text in the language-list modal search field.
    pub services_langs_search: String,

    pub toasts: Vec<Toast>,

    logo_texture: Option<egui::TextureHandle>,
}

impl TranslatorApp {
    fn apply_theme(ctx: &egui::Context) {
        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.indent = 18.0;
        style.visuals.button_frame = true;
        style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(
            1.0,
            egui::Color32::from_rgb(50, 55, 70),
        );
        ctx.set_style(style);

        let mut theme = egui::Visuals::dark();
        theme.panel_fill = egui::Color32::from_rgb(24, 26, 34);
        theme.window_fill = egui::Color32::from_rgb(30, 32, 42);
        theme.extreme_bg_color = egui::Color32::from_rgb(18, 20, 28);
        theme.faint_bg_color = egui::Color32::from_rgb(30, 33, 44);
        theme.hyperlink_color = egui::Color32::from_rgb(90, 142, 242);
        theme.selection.bg_fill = egui::Color32::from_rgb(50, 90, 160);

        theme.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(180));
        theme.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(30, 33, 44);

        theme.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
        theme.widgets.hovered.bg_fill = egui::Color32::from_rgb(50, 55, 75);
        theme.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(90, 142, 242));

        theme.widgets.active.fg_stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
        theme.widgets.active.bg_fill = egui::Color32::from_rgb(55, 60, 82);
        theme.widgets.active.bg_stroke = egui::Stroke::new(1.5, egui::Color32::from_rgb(90, 142, 242));

        theme.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(180));
        theme.widgets.inactive.bg_fill = egui::Color32::from_rgb(35, 38, 50);
        theme.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(50, 55, 70));

        ctx.set_visuals(theme);
    }

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
        Self::apply_theme(&cc.egui_ctx);

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
            detect_start_time: None,
            update_state: UpdateState::Idle,
            update_check_result: Arc::new(Mutex::new(None)),
            update_download_result: Arc::new(Mutex::new(None)),
            update_progress: Arc::new(Mutex::new(0.0)),
            show_update_dialog: false,
            credits_balance: None,
            credits_packages: Vec::new(),
            credits_state: CreditsLoadState::Idle,
            credits_error: String::new(),
            services_info: Vec::new(),
            services_state: CreditsLoadState::Idle,
            services_error: String::new(),
            selected_service_idx: None,
            services_langs_modal_idx: None,
            services_langs_search: String::new(),
            toasts: Vec::new(),
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

    pub fn toast(&mut self, message: impl Into<String>, color: egui::Color32) {
        self.toasts.push(Toast {
            message: message.into(),
            color,
            created_at: std::time::Instant::now(),
        });
    }

    pub fn draw_toasts(&mut self, ctx: &egui::Context) {
        let now = std::time::Instant::now();
        self.toasts.retain(|t| now.duration_since(t.created_at) < TOAST_DURATION);

        if self.toasts.is_empty() {
            return;
        }

        let screen = ctx.screen_rect();
        let mut y = screen.bottom() - 20.0;

        for toast in &self.toasts {
            let elapsed = now.duration_since(toast.created_at);
            let remaining = TOAST_DURATION - elapsed;
            let alpha = if remaining < TOAST_FADE {
                (remaining.as_millis() as f32 / TOAST_FADE.as_millis() as f32 * 255.0) as u8
            } else {
                255
            };

            let galley = ctx.fonts(|f| f.layout_no_wrap(
                toast.message.clone(),
                egui::FontId::proportional(13.0),
                egui::Color32::WHITE,
            ));
            let text_width = galley.size().x;
            let pad_x = 16.0_f32;
            let pad_y = 8.0_f32;
            let total_w = text_width + pad_x * 2.0;
            let total_h = galley.size().y + pad_y * 2.0;
            y -= total_h + 6.0;

            let pos = egui::pos2(
                screen.center().x - total_w / 2.0,
                y,
            );

            let bg_color = egui::Color32::from_rgba_premultiplied(30, 32, 42, alpha);
            let border_color = egui::Color32::from_rgba_premultiplied(
                toast.color.r(),
                toast.color.g(),
                toast.color.b(),
                alpha,
            );
            let text_color = egui::Color32::from_rgba_premultiplied(
                toast.color.r(),
                toast.color.g(),
                toast.color.b(),
                alpha,
            );

            egui::Area::new(egui::Id::new(("toast", toast.created_at)))
                .order(egui::Order::Foreground)
                .fixed_pos(pos)
                .interactable(false)
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .corner_radius(8.0)
                        .fill(bg_color)
                        .stroke(egui::Stroke::new(1.0, border_color))
                        .inner_margin(egui::Margin::symmetric(pad_x as i8, pad_y as i8))
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(&toast.message).color(text_color).size(13.0));
                        });
                });
        }
    }

    fn write_result_on_close(&self, ctx: &egui::Context) {
        let is_closing = ctx.input(|i| i.viewport().close_requested());
        if is_closing {
            self.write_result();
        }
    }

    fn draw_tab_bar(&mut self, ui: &mut egui::Ui) {
        let tab_accent = egui::Color32::from_rgb(90, 142, 242);
        let tab_bg_active = egui::Color32::from_rgb(50, 60, 80);
        let tab_border = egui::Color32::from_rgb(60, 66, 80);

        let tabs = [
            (Tab::Translate, "Translate"),
            (Tab::AiModels, "AI Models"),
            (Tab::Settings, "Settings"),
            (Tab::Account, "Account"),
            (Tab::Credits, "Credits"),
        ];

        ui.horizontal(|ui| {
            ui.add_space(4.0);
            for (tab, label) in tabs {
                let is_active = self.active_tab == tab;

                let frame = egui::Frame::group(ui.style())
                    .inner_margin(egui::Margin::symmetric(10, 5))
                    .corner_radius(6.0)
                    .stroke(egui::Stroke::new(
                        if is_active { 1.5 } else { 0.0 },
                        if is_active { tab_accent } else { tab_border },
                    ))
                    .fill(if is_active { tab_bg_active } else { egui::Color32::TRANSPARENT });

                let resp = frame.show(ui, |ui| {
                    let text = egui::RichText::new(label).color(if is_active {
                        tab_accent
                    } else {
                        egui::Color32::from_gray(180)
                    });
                    let label = if is_active {
                        egui::Label::new(text.strong())
                    } else {
                        egui::Label::new(text)
                    };
                    ui.add(label.sense(egui::Sense::click()))
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                });

                if resp.inner.clicked() {
                    self.active_tab = tab;
                }

                ui.add_space(3.0);
            }
        });

        ui.add_space(4.0);
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
        ui.painter().line_segment(
            [
                egui::pos2(rect.left(), rect.center().y),
                egui::pos2(rect.right(), rect.center().y),
            ],
            egui::Stroke::new(1.0, tab_border),
        );
        ui.add_space(6.0);
    }
}

impl eframe::App for TranslatorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.first_frame {
            self.first_frame = false;
            self.write_result();
            if self.settings.has_credentials() && matches!(self.update_state, UpdateState::Idle) {
                self.check_for_updates(ctx.clone());
            }
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

        if matches!(self.update_state, UpdateState::Downloading) {
            self.process_download_result();
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        // Auto-fetch credits when user opens the Credits tab for the first time
        // (or after a previous error). Subsequent refreshes are user-driven.
        if self.active_tab == Tab::Credits
            && self.settings.has_credentials()
            && matches!(self.credits_state, CreditsLoadState::Idle)
        {
            self.refresh_credits();
            ctx.request_repaint();
        }

        if matches!(self.credits_state, CreditsLoadState::Loading) {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        // Auto-fetch services info when user opens the AI Models tab for the first
        // time (or after a previous error). Subsequent refreshes are user-driven.
        if self.active_tab == Tab::AiModels
            && self.settings.has_credentials()
            && matches!(self.services_state, CreditsLoadState::Idle)
        {
            self.refresh_services_info();
            ctx.request_repaint();
        }

        if matches!(self.services_state, CreditsLoadState::Loading) {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
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

        if matches!(self.services_state, CreditsLoadState::Error) {
            self.draw_services_error_dialog(ctx);
        }

        if self.services_langs_modal_idx.is_some() {
            self.draw_services_langs_modal(ctx);
        }

        if self.detecting_language {
            self.draw_detect_dialog(ctx);
        }

        self.draw_toasts(ctx);

        self.write_result_on_close(ctx);
    }
}

impl Drop for TranslatorApp {
    fn drop(&mut self) {
        self.write_result();
    }
}
