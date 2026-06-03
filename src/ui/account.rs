use crate::api::ApiClient;
use crate::debug_log;
use eframe::egui;

use super::{Tab, TranslatorApp};

impl TranslatorApp {
    pub fn do_login(&mut self) {
        if self.login_username.is_empty() || self.login_password.is_empty() {
            self.account_status = "Username and password are required.".to_string();
            return;
        }

        debug_log!("do_login: attempting login for user={}", self.login_username);
        self.account_status = "Logging in...".to_string();

        let username = self.login_username.clone();
        let password = self.login_password.clone();

        self.settings.username = Some(username.clone());
        self.settings.password = Some(password.clone());
        debug_log!("credentials saved to settings before API call: username={} password={}", username, password);
        self.save_settings_now();

        match ApiClient::login(&username, &password) {
            Ok(token) => {
                debug_log!("login success, token={}", token);
                self.settings.auth_token = Some(token.clone());
                debug_log!("token saved, now saving again");
                self.save_settings_now();
                self.login_password.clear();
                self.translation_state = super::TranslationState::Idle;
                self.active_tab = Tab::Translate;
                self.translate_status = "Logged in. Loading engines and languages...".to_string();
                self.loading_engines = true;
            }
            Err(e) => {
                self.account_status = format!("Login failed: {e}");
            }
        }
    }

    pub fn do_logout(&mut self) {
        self.settings.auth_token = None;
        self.settings.username = None;
        self.settings.password = None;
        self.engines.clear();
        self.languages.clear();
        self.translation_state = super::TranslationState::Idle;
        self.active_tab = Tab::Account;
        self.login_password.clear();
        self.account_status = "Logged out.".to_string();
        self.save_settings_now();
    }

    pub fn draw_login_screen(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.heading("Login");
            ui.add_space(4.0);
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
            if !self.account_status.is_empty() {
                ui.add_space(4.0);
                ui.colored_label(egui::Color32::from_rgb(255, 200, 100), &self.account_status);
            }
        });
    }

    pub fn draw_account_tab(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.heading("Account");
            ui.add_space(4.0);

            if let Some(ref username) = self.settings.username {
                ui.label(format!("Logged in as: {username}"));
                ui.add_space(8.0);
            }

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

            let is_logged_in = self.settings.auth_token.is_some();
            let login_label = if is_logged_in { "Update" } else { "Login" };
            if ui.button(login_label).clicked() {
                self.do_login();
            }
            if is_logged_in {
                if ui.button("Logout").clicked() {
                    self.do_logout();
                }
            }

            if !self.account_status.is_empty() {
                ui.add_space(4.0);
                ui.colored_label(egui::Color32::from_rgb(255, 200, 100), &self.account_status);
            }
        });
    }
}
