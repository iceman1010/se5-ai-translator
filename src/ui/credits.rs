use crate::api::{ApiClient, CreditPackage};
use crate::debug_log;
use eframe::egui;

use super::{CreditsLoadState, TranslatorApp};

impl TranslatorApp {
    /// Fetches fresh credit balance and package list from the API.
    /// Called on user request, after login, and when the Credits tab is opened with stale data.
    pub fn refresh_credits(&mut self) {
        let Some(token) = self.settings.auth_token.clone() else {
            self.credits_state = CreditsLoadState::Error;
            self.credits_error = "Not logged in.".to_string();
            return;
        };

        self.credits_state = CreditsLoadState::Loading;
        self.credits_error.clear();

        let client = ApiClient::new(&token);

        // Balance
        match client.get_credits() {
            Ok(balance) => {
                debug_log!("refresh_credits: balance={balance}");
                self.credits_balance = Some(balance);
            }
            Err(e) => {
                debug_log!("refresh_credits: balance error: {e}");
                self.credits_state = CreditsLoadState::Error;
                self.credits_error = format!("Failed to load balance: {e}");
                return;
            }
        }

        // Packages
        match client.get_credit_packages(None) {
            Ok(packages) => {
                debug_log!("refresh_credits: {} packages", packages.len());
                self.credits_packages = packages;
            }
            Err(e) => {
                debug_log!("refresh_credits: packages error: {e}");
                // Balance succeeded but packages failed — keep balance visible.
                self.credits_state = CreditsLoadState::Error;
                self.credits_error = format!("Failed to load packages: {e}");
                return;
            }
        }

        self.credits_state = CreditsLoadState::Loaded;
    }

    pub fn draw_credits_tab(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.heading("Credits");
            ui.add_space(8.0);

            // Top row: balance + refresh button
            ui.horizontal(|ui| {
                let balance_text = match self.credits_balance {
                    Some(b) => format!("{b:.2}"),
                    None => "—".to_string(),
                };
                ui.label(egui::RichText::new("Balance:").strong());
                ui.label(egui::RichText::new(&balance_text).size(18.0).strong().color(
                    egui::Color32::from_rgb(120, 200, 120),
                ));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let refreshing = matches!(self.credits_state, CreditsLoadState::Loading);
                    let btn = egui::Button::new(if refreshing { "Refreshing..." } else { "Refresh" });
                    if ui.add_enabled(!refreshing, btn).clicked() {
                        self.refresh_credits();
                    }
                });
            });

            // Status messages
            match self.credits_state {
                CreditsLoadState::Loading => {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Loading...").color(egui::Color32::from_rgb(200, 200, 120)));
                }
                CreditsLoadState::Error if !self.credits_error.is_empty() => {
                    ui.add_space(4.0);
                    ui.colored_label(egui::Color32::from_rgb(255, 120, 120), &self.credits_error);
                }
                _ => {}
            }

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);

            // Low-balance warning
            if let Some(b) = self.credits_balance
                && b < 10.0
            {
                ui.add_space(4.0);
                ui.colored_label(
                    egui::Color32::from_rgb(255, 180, 80),
                    format!("Low balance ({b:.2}). Consider purchasing more credits below."),
                );
                ui.add_space(4.0);
            }

            // Packages section
            ui.heading("Buy credits");
            ui.add_space(4.0);

            if self.credits_packages.is_empty() {
                ui.label(
                    egui::RichText::new("No packages available.")
                        .color(egui::Color32::GRAY),
                );
            } else {
                egui::Grid::new("credit_packages_grid")
                    .striped(true)
                    .min_col_width(120.0)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("Package").strong());
                        ui.label(egui::RichText::new("Price").strong());
                        ui.label(egui::RichText::new("Discount").strong());
                        ui.label(egui::RichText::new("Action").strong());
                        ui.end_row();

                        for pkg in &self.credits_packages {
                            self.draw_package_row(ui, pkg);
                        }
                    });
            }
        });
    }

    fn draw_package_row(&self, ui: &mut egui::Ui, pkg: &CreditPackage) {
        ui.label(&pkg.name);
        // The API returns `value` as a price string like "5 USD" — display it as-is.
        ui.label(&pkg.value);

        let discount_label = if pkg.discount_percent > 0.0 {
            format!("{:.0}%", pkg.discount_percent)
        } else {
            "—".to_string()
        };
        let discount_color = if pkg.discount_percent > 0.0 {
            egui::Color32::from_rgb(120, 200, 120)
        } else {
            egui::Color32::GRAY
        };
        ui.label(egui::RichText::new(discount_label).color(discount_color));

        if ui.button("Buy").clicked() {
            debug_log!(
                "opening checkout URL for {} ({} credits): {}",
                pkg.name,
                pkg.credit_count().map_or("?".to_string(), |n| n.to_string()),
                pkg.checkout_url
            );
            if let Err(e) = open_url(&pkg.checkout_url) {
                debug_log!("failed to open URL: {e}");
            }
        }
        ui.end_row();
    }
}

#[allow(clippy::needless_return)]
fn open_url(url: &str) -> Result<(), String> {
    // open::open is not depended on; use a platform-specific shell command to keep deps minimal.
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
    {
        let _ = url;
        Err("URL opening is not supported on this platform".to_string())
    }
}
