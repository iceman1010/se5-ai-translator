use eframe::egui;
use std::sync::{Arc, Mutex};

use super::TranslatorApp;

pub enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    UpdateAvailable {
        latest_version: String,
        html_url: String,
    },
    Error(String),
}

pub type UpdateCheckResult = Arc<Mutex<Option<Result<Option<(String, String)>, String>>>>;

fn parse_version_parts(version: &str) -> Vec<u32> {
    version
        .split('.')
        .filter_map(|s| s.parse::<u32>().ok())
        .collect()
}

impl TranslatorApp {
    pub fn check_for_updates(&mut self, ctx: egui::Context) {
        self.update_state = UpdateState::Checking;
        let result = self.update_check_result.clone();

        std::thread::spawn(move || {
            let client = reqwest::blocking::Client::builder()
                .user_agent("se-ai-translator")
                .timeout(std::time::Duration::from_secs(10))
                .build();

            let outcome = match client {
                Ok(client) => {
                    let api_url = format!(
                        "{}/releases/latest",
                        super::REPO_URL
                            .trim_end_matches('/')
                            .replace("https://github.com/", "https://api.github.com/repos/")
                    );
                    match client.get(&api_url).header("Accept", "application/vnd.github+json").send() {
                        Ok(resp) => {
                            match resp.json::<serde_json::Value>() {
                                Ok(json) => {
                                    let tag = json.get("tag_name").and_then(|v| v.as_str()).unwrap_or("");
                                    let html_url = json.get("html_url").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    let version = tag.trim_start_matches('v').to_string();

                                    let latest = parse_version_parts(&version);
                                    let current = parse_version_parts(env!("CARGO_PKG_VERSION"));

                                    if latest > current {
                                        Ok(Some((version, html_url)))
                                    } else {
                                        Ok(None)
                                    }
                                }
                                Err(e) => Err(format!("Failed to parse release info: {e}")),
                            }
                        }
                        Err(e) => Err(format!("Failed to check for updates: {e}")),
                    }
                }
                Err(e) => Err(format!("Failed to create HTTP client: {e}")),
            };

            if let Ok(mut res) = result.lock() {
                *res = Some(outcome);
            }
            ctx.request_repaint();
        });
    }

    pub fn process_update_result(&mut self) {
        if let Ok(mut res) = self.update_check_result.lock() {
            if let Some(outcome) = res.take() {
                match outcome {
                    Ok(Some((version, html_url))) => {
                        self.update_state = UpdateState::UpdateAvailable {
                            latest_version: version,
                            html_url,
                        };
                        self.show_update_dialog = true;
                    }
                    Ok(None) => {
                        self.update_state = UpdateState::UpToDate;
                    }
                    Err(e) => {
                        self.update_state = UpdateState::Error(e);
                    }
                }
            }
        }
    }

    pub fn draw_settings_tab(&mut self, ui: &mut egui::Ui, ctx: egui::Context) {
        ui.group(|ui| {
            ui.heading("Settings");
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.label(format!("Current version: {}", env!("CARGO_PKG_VERSION")));
            });
            ui.add_space(8.0);

            match &self.update_state {
                UpdateState::Checking => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Checking for updates...");
                    });
                }
                UpdateState::UpToDate => {
                    ui.label(egui::RichText::new("You are up to date!").color(egui::Color32::GREEN));
                    ui.add_space(4.0);
                    if ui.button("Check Again").clicked() {
                        self.check_for_updates(ctx);
                    }
                }
                UpdateState::UpdateAvailable { latest_version, .. } => {
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 180, 60),
                        format!("New version available: {latest_version}"),
                    );
                    ui.add_space(4.0);
                    if ui.button("Download Update").clicked() {
                        self.show_update_dialog = true;
                    }
                }
                UpdateState::Error(e) => {
                    ui.colored_label(egui::Color32::RED, format!("Update check failed: {e}"));
                    ui.add_space(4.0);
                    if ui.button("Retry").clicked() {
                        self.check_for_updates(ctx);
                    }
                }
                UpdateState::Idle => {
                    if ui.button("Check for Updates").clicked() {
                        self.check_for_updates(ctx);
                    }
                }
            }
        });
    }

    pub fn draw_update_dialog(&mut self, ctx: &egui::Context) {
        let (latest_version, html_url) = match &self.update_state {
            UpdateState::UpdateAvailable { latest_version, html_url } => {
                (latest_version.clone(), html_url.clone())
            }
            _ => {
                self.show_update_dialog = false;
                return;
            }
        };

        let current = env!("CARGO_PKG_VERSION").to_string();
        let title = format!("Update Available - v{latest_version}");

        egui::Window::new(&title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label("A new version is available!");
                    ui.add_space(8.0);
                    ui.label(format!("Current: v{current}"));
                    ui.label(format!("Latest:  v{latest_version}"));
                    ui.add_space(12.0);
                    ui.label("The download page will open in your browser.");
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button("Download").clicked() {
                            let _ = open::that(&html_url);
                            self.show_update_dialog = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.show_update_dialog = false;
                        }
                    });
                });
            });
    }
}
