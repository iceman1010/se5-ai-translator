use crate::api::ApiClient;
use crate::debug_log;
use eframe::egui;
use egui_extras::{Column, TableBuilder};

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
        // The parent uses `vertical_centered`, which sizes children to their content
        // rather than stretching them to the panel width. Without this allocation,
        // the table's `Column::remainder()` has nothing to absorb and resizing the
        // window wouldn't resize the table.
        let full_w = ui.available_width();
        ui.allocate_ui(egui::Vec2::new(full_w, ui.available_height()), |ui| {
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
                    self.draw_packages_table(ui);
                }
            });
        });
    }

    /// Dynamic-width table of credit packages.
    ///
    /// Layout: Package column is `remainder()` (absorbs extra width when the window
    /// is wide; triggers horizontal scrolling when too narrow). The other columns
    /// are sized to their content. Built with `egui_extras::TableBuilder` so the
    /// same pattern can be reused for any future tables.
    ///
    /// Cell layout is `left_to_right(Align::Center)`, NOT `centered_and_justified`.
    /// The latter sets `main_justify = true`, which stretches every cell's contents
    /// to fill the cell width. egui_extras records that inflated width as the
    /// column's `max_used_width`, and `TableState::load` (table.rs:646) then uses
    /// `max(width_range.min, max_used)` as a non-shrinking floor — making the table
    /// grow when the window widens but refuse to shrink when it narrows.
    fn draw_packages_table(&self, ui: &mut egui::Ui) {
        let table = TableBuilder::new(ui)
            .striped(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .min_scrolled_height(0.0)
            .max_scroll_height(f32::INFINITY)
            .column(Column::remainder().at_least(140.0))
            .column(Column::auto().at_least(70.0))
            .column(Column::auto().at_least(80.0))
            .column(Column::exact(70.0));

        table
            .header(24.0, |mut header| {
                header.col(|ui| { ui.strong("Package"); });
                header.col(|ui| { ui.strong("Price"); });
                header.col(|ui| { ui.strong("Discount"); });
                header.col(|ui| { ui.strong("Action"); });
            })
            .body(|mut body| {
                let row_h = 24.0_f32;

                for pkg in &self.credits_packages {
                    body.row(row_h, |mut row| {
                        row.col(|ui| { ui.label(&pkg.name); });
                        // API returns `value` as a price string like "5 USD" — display as-is.
                        row.col(|ui| { ui.label(&pkg.value); });

                        row.col(|ui| {
                            let has_discount = pkg.discount_percent > 0.0;
                            let text = if has_discount {
                                format!("{:.0}%", pkg.discount_percent)
                            } else {
                                "—".to_string()
                            };
                            let color = if has_discount {
                                egui::Color32::from_rgb(120, 200, 120)
                            } else {
                                egui::Color32::GRAY
                            };
                            ui.label(egui::RichText::new(text).color(color));
                        });

                        row.col(|ui| {
                            if ui.button("Buy").clicked() {
                                debug_log!(
                                    "opening checkout URL for {} ({} credits): {}",
                                    pkg.name,
                                    pkg.credit_count().map_or("?".to_string(), |n| n.to_string()),
                                    pkg.checkout_url
                                );
                                if let Err(e) = open::that(&pkg.checkout_url) {
                                    debug_log!("failed to open URL: {e}");
                                }
                            }
                        });
                    });
                }
            });
    }
}
