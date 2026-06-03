use eframe::egui;
use std::io::Read;
use std::sync::{Arc, Mutex};

use super::TranslatorApp;

pub enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    UpdateAvailable {
        latest_version: String,
        download_url: String,
    },
    Downloading {
        progress: f32,
    },
    Error(String),
}

pub type UpdateCheckResult = Arc<Mutex<Option<Result<Option<(String, String)>, String>>>>;
pub type UpdateDownloadResult = Arc<Mutex<Option<Result<(), String>>>>;

fn parse_version_parts(version: &str) -> Vec<u32> {
    version
        .split('.')
        .filter_map(|s| s.parse::<u32>().ok())
        .collect()
}

fn platform_artifact_name() -> String {
    let os = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };

    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    };

    let ext = if cfg!(target_os = "windows") {
        "zip"
    } else {
        "tar.gz"
    };

    format!("se-ai-translator-{os}-{arch}.{ext}")
}

fn binary_name_in_archive() -> String {
    if cfg!(target_os = "windows") {
        "se-ai-translator.exe".to_string()
    } else {
        "se-ai-translator".to_string()
    }
}

fn extract_binary(archive_bytes: &[u8]) -> Result<std::path::PathBuf, String> {
    let temp_dir = std::env::temp_dir().join("se-ai-translator-update");
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("Failed to create temp dir: {e}"))?;

    let binary_name = binary_name_in_archive();
    let output_path = temp_dir.join(&binary_name);

    if cfg!(target_os = "windows") {
        let reader = std::io::Cursor::new(archive_bytes);
        let mut archive = zip::ZipArchive::new(reader)
            .map_err(|e| format!("Failed to open zip: {e}"))?;

        let mut file = archive.by_name(&binary_name)
            .map_err(|e| format!("Binary not found in zip: {e}"))?;

        let mut out = std::fs::File::create(&output_path)
            .map_err(|e| format!("Failed to create temp file: {e}"))?;

        std::io::copy(&mut file, &mut out)
            .map_err(|e| format!("Failed to extract binary: {e}"))?;
    } else {
        let reader = std::io::Cursor::new(archive_bytes);
        let gz = flate2::read::GzDecoder::new(reader);
        let mut archive = tar::Archive::new(gz);

        let mut found = false;
        for entry in archive.entries().map_err(|e| format!("Failed to read tar: {e}"))? {
            let mut entry = entry.map_err(|e| format!("Failed to read tar entry: {e}"))?;
            let path = entry.path().map_err(|e| format!("Bad path: {e}"))?;
            if path.file_name().map(|n| n == binary_name.as_str()).unwrap_or(false) {
                let mut out = std::fs::File::create(&output_path)
                    .map_err(|e| format!("Failed to create temp file: {e}"))?;
                std::io::copy(&mut entry, &mut out)
                    .map_err(|e| format!("Failed to extract binary: {e}"))?;

                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&output_path, std::fs::Permissions::from_mode(0o755))
                        .map_err(|e| format!("Failed to set permissions: {e}"))?;
                }

                found = true;
                break;
            }
        }

        if !found {
            return Err(format!("Binary '{binary_name}' not found in archive"));
        }
    }

    Ok(output_path)
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
                                    let version = tag.trim_start_matches('v').to_string();

                                    let latest = parse_version_parts(&version);
                                    let current = parse_version_parts(env!("CARGO_PKG_VERSION"));

                                    if latest > current {
                                        let artifact = platform_artifact_name();
                                        let download_url = json.get("assets")
                                            .and_then(|a| a.as_array())
                                            .and_then(|assets| assets.iter().find(|a| {
                                                a.get("name").and_then(|n| n.as_str()).unwrap_or("") == artifact
                                            }))
                                            .and_then(|a| a.get("browser_download_url").and_then(|u| u.as_str()))
                                            .unwrap_or("")
                                            .to_string();

                                        if download_url.is_empty() {
                                            Err(format!("No matching artifact found for platform: {artifact}"))
                                        } else {
                                            Ok(Some((version, download_url)))
                                        }
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
                    Ok(Some((version, download_url))) => {
                        self.update_state = UpdateState::UpdateAvailable {
                            latest_version: version,
                            download_url,
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

    pub fn start_self_update(&mut self, ctx: egui::Context) {
        let download_url = match &self.update_state {
            UpdateState::UpdateAvailable { download_url, .. } => download_url.clone(),
            _ => return,
        };

        self.update_state = UpdateState::Downloading { progress: 0.0 };
        let result = self.update_download_result.clone();

        std::thread::spawn(move || {
            let outcome = (|| -> Result<(), String> {
                let client = reqwest::blocking::Client::builder()
                    .user_agent("se-ai-translator")
                    .timeout(std::time::Duration::from_secs(120))
                    .build()
                    .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

                let mut resp = client.get(&download_url)
                    .send()
                    .map_err(|e| format!("Failed to download update: {e}"))?;

                let total = resp.content_length().unwrap_or(0) as usize;
                let mut archive_data = Vec::with_capacity(total.max(1024 * 1024));

                let chunk_size = 64 * 1024;
                loop {
                    let mut chunk = Vec::with_capacity(chunk_size);
                    let n = Read::by_ref(&mut resp)
                        .take(chunk_size as u64)
                        .read_to_end(&mut chunk)
                        .map_err(|e| format!("Download error: {e}"))?;

                    if n == 0 { break; }
                    archive_data.extend_from_slice(&chunk);
                }

                let temp_binary = extract_binary(&archive_data)?;

                self_replace::self_replace(&temp_binary)
                    .map_err(|e| format!("Failed to replace binary: {e}"))?;

                let _ = std::fs::remove_file(&temp_binary);

                Ok(())
            })();

            if let Ok(mut res) = result.lock() {
                *res = Some(outcome);
            }
            ctx.request_repaint();
        });
    }

    pub fn process_download_result(&mut self) {
        if let Ok(mut res) = self.update_download_result.lock() {
            if let Some(outcome) = res.take() {
                match outcome {
                    Ok(()) => {
                        self.update_state = UpdateState::UpToDate;
                        self.show_update_dialog = false;
                    }
                    Err(e) => {
                        self.update_state = UpdateState::Error(e);
                        self.show_update_dialog = false;
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
                UpdateState::Downloading { progress } => {
                    ui.label("Downloading update...");
                    ui.add_space(4.0);
                    ui.add(
                        egui::ProgressBar::new(*progress)
                            .show_percentage()
                            .animate(true),
                    );
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
                    if ui.button("Update Now").clicked() {
                        self.start_self_update(ctx);
                    }
                }
                UpdateState::Error(e) => {
                    ui.colored_label(egui::Color32::RED, format!("Update failed: {e}"));
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
        let (latest_version, download_url) = match &self.update_state {
            UpdateState::UpdateAvailable { latest_version, download_url } => {
                (latest_version.clone(), download_url.clone())
            }
            UpdateState::Downloading { .. } => {
                egui::Window::new("Updating...")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.spinner();
                            ui.label("Downloading and installing update...");
                            ui.add_space(8.0);
                            ui.label("The plugin will restart automatically.");
                        });
                    });
                return;
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
                    ui.label("The plugin will update and restart.");
                    ui.add_space(12.0);

                    let mut do_update = false;
                    ui.horizontal(|ui| {
                        if ui.button("Update Now").clicked() {
                            do_update = true;
                        }
                        if ui.button("Cancel").clicked() {
                            self.show_update_dialog = false;
                        }
                    });

                    if do_update {
                        self.start_self_update(ctx.clone());
                    }

                    let _ = download_url;
                });
            });
    }
}
