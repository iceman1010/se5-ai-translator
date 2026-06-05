use crate::api::ApiClient;
use crate::debug_log;
use crate::ui::update::{paint_backdrop, ui_style_for_modal};
use eframe::egui;
use egui_extras::{Column, TableBuilder};

use super::{CreditsLoadState, TranslatorApp};

impl TranslatorApp {
    /// Fetches the list of AI translation models from `GET /ai/info/services`.
    /// Called on first tab open and on user-initiated refresh (via the error
    /// modal's retry button).
    pub fn refresh_services_info(&mut self) {
        let Some(token) = self.settings.auth_token.clone() else {
            self.services_state = CreditsLoadState::Error;
            self.services_error = "Not logged in.".to_string();
            return;
        };

        self.services_state = CreditsLoadState::Loading;
        self.services_error.clear();

        let client = ApiClient::new(&token);
        match client.get_services_info() {
            Ok(models) => {
                debug_log!("refresh_services_info: {} translation models", models.len());
                if models.is_empty() {
                    // Empty result is suspicious — most likely a deserialization
                    // mismatch (e.g. field names changed). Log it loudly so it
                    // doesn't look like "no models available" with no clue.
                    debug_log!("refresh_services_info: WARNING — empty translation list, possible serde mismatch");
                }
                self.services_info = models;
                // Keep current selection if still valid, else select the first row.
                if let Some(idx) = self.selected_service_idx {
                    if idx >= self.services_info.len() {
                        self.selected_service_idx = if self.services_info.is_empty() {
                            None
                        } else {
                            Some(0)
                        };
                    }
                } else if !self.services_info.is_empty() {
                    self.selected_service_idx = Some(0);
                }
                self.services_state = CreditsLoadState::Loaded;
            }
            Err(e) => {
                debug_log!("refresh_services_info error: {e}");
                self.services_state = CreditsLoadState::Error;
                self.services_error = format!("Failed to load models: {e}");
            }
        }
    }

    pub fn draw_ai_models_tab(&mut self, ui: &mut egui::Ui) {
        // The parent uses `vertical_centered`, which sizes children to their content
        // rather than stretching them to the panel width. Without this allocation,
        // the table's `Column::remainder()` has nothing to absorb and resizing the
        // window wouldn't resize the table. Same pattern as credits tab.
        let full_w = ui.available_width();
        ui.allocate_ui(egui::Vec2::new(full_w, ui.available_height()), |ui| {
            ui.group(|ui| {
                ui.heading("AI Translation Models");
                ui.add_space(8.0);

                match self.services_state {
                    CreditsLoadState::Loading => {
                        ui.label(
                            egui::RichText::new("Loading models...")
                                .color(egui::Color32::from_rgb(200, 200, 120)),
                        );
                        ui.add_space(4.0);
                        ui.spinner();
                    }
                    CreditsLoadState::Loaded if self.services_info.is_empty() => {
                        ui.label(
                            egui::RichText::new("No models available.")
                                .color(egui::Color32::GRAY),
                        );
                    }
                    CreditsLoadState::Loaded => {
                        let count = self.services_info.len();
                        ui.label(
                            egui::RichText::new(format!("{count} model(s) available"))
                                .color(egui::Color32::from_gray(170))
                                .small(),
                        );
                    }
                    // Error state: the inline UI is empty here; the error is
                    // presented as a modal by `draw_services_error_dialog`.
                    CreditsLoadState::Idle | CreditsLoadState::Error => {}
                }

                if matches!(self.services_state, CreditsLoadState::Loaded) && !self.services_info.is_empty() {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(4.0);
                    self.draw_models_table(ui);
                    ui.add_space(8.0);
                    self.draw_selected_model_details(ui);
                }
            });
        });
    }

    /// Modal shown when fetching services info failed. Gives the user a Retry
    /// button (which re-runs `refresh_services_info`) and a Cancel button that
    /// just dismisses the modal (state goes back to Idle so the next tab open
    /// retries automatically).
    pub fn draw_services_error_dialog(&mut self, ctx: &egui::Context) {
        if !matches!(self.services_state, CreditsLoadState::Error) {
            return;
        }
        // Empty error string = login error path; show that too.
        if self.services_error.is_empty() {
            return;
        }

        paint_backdrop(ctx);

        let err = self.services_error.clone();
        let mut do_retry = false;
        let mut do_cancel = false;

        egui::Window::new("Loading failed")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(
                egui::Frame::window(&ui_style_for_modal(ctx))
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
                    ui.label(
                        egui::RichText::new("Could not load AI models")
                            .color(egui::Color32::from_rgb(255, 120, 120))
                            .strong(),
                    );
                    ui.add_space(8.0);
                });

                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(&err)
                        .color(egui::Color32::from_gray(200))
                        .small(),
                );
                ui.add_space(14.0);

                ui.horizontal(|ui| {
                    if ui.button("Retry").clicked() {
                        do_retry = true;
                    }
                    if ui.button("Cancel").clicked() {
                        do_cancel = true;
                    }
                });
            });

        if do_retry {
            self.refresh_services_info();
        }
        if do_cancel {
            // Drop back to Idle so the next visit to the tab will retry, and
            // clear the error so the modal doesn't immediately reappear.
            self.services_state = CreditsLoadState::Idle;
            self.services_error.clear();
        }
    }

    /// Sortable table of translation models. Uses the same scrolling pattern as
    /// the credits packages table (`min_scrolled_height(0.0)` +
    /// `max_scroll_height(INFINITY)` + `cell_layout(left_to_right)`) so the table
    /// grows with the window and scrolls in both directions when needed.
    fn draw_models_table(&mut self, ui: &mut egui::Ui) {
        // Snapshot the row data we need up-front. The table body closure takes
        // `&mut self` indirectly through `row.col()`, so capturing the data here
        // avoids borrow-checker conflicts and lets us cleanly handle row clicks.
        let rows: Vec<RowData> = self
            .services_info
            .iter()
            .enumerate()
            .map(|(i, m)| RowData {
                idx: i,
                display_name: m.display_name.clone(),
                reliability: m.reliability.clone(),
                speed: m.speed.clone(),
                price_per_1k: m.price_per_1000(),
                lang_count: m.languages_supported.len(),
            })
            .collect();

        let selected = self.selected_service_idx;
        let mut new_selection: Option<usize> = None;

        let table = TableBuilder::new(ui)
            .striped(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .min_scrolled_height(0.0)
            .max_scroll_height(f32::INFINITY)
            .column(Column::remainder().at_least(160.0))
            .column(Column::auto().at_least(80.0))
            .column(Column::auto().at_least(80.0))
            .column(Column::auto().at_least(90.0))
            .column(Column::auto().at_least(70.0));

        table
            .header(24.0, |mut header| {
                header.col(|ui| { ui.strong("Model"); });
                header.col(|ui| { ui.strong("Reliability"); });
                header.col(|ui| { ui.strong("Speed"); });
                header.col(|ui| { ui.strong("Price / 1k"); });
                header.col(|ui| { ui.strong("Languages"); });
            })
            .body(|mut body| {
                let row_h = 22.0_f32;
                for row_data in &rows {
                    let is_selected = selected == Some(row_data.idx);
                    let mut clicked_this_row = false;
                    body.row(row_h, |mut row| {
                        // Model name
                        row.col(|ui| {
                            let text = egui::RichText::new(&row_data.display_name)
                                .strong()
                                .color(if is_selected {
                                    egui::Color32::from_rgb(90, 142, 242)
                                } else {
                                    egui::Color32::from_gray(220)
                                });
                            ui.label(text);
                        });

                        // Reliability (colored)
                        row.col(|ui| {
                            let color = reliability_color(&row_data.reliability);
                            ui.label(egui::RichText::new(&row_data.reliability).color(color));
                        });

                        // Speed
                        row.col(|ui| {
                            ui.label(
                                egui::RichText::new(&row_data.speed)
                                    .color(egui::Color32::from_gray(200)),
                            );
                        });

                        // Price per 1k chars
                        row.col(|ui| {
                            ui.label(format!("${:.4}", row_data.price_per_1k));
                        });

                        // Language count
                        row.col(|ui| {
                            ui.label(format!("{}", row_data.lang_count));
                        });

                        // Click anywhere on the row to select it.
                        if row.response().clicked() {
                            clicked_this_row = true;
                        }
                    });

                    if clicked_this_row {
                        new_selection = Some(row_data.idx);
                    }
                }
            });

        if let Some(idx) = new_selection {
            self.selected_service_idx = Some(idx);
            self.services_langs_expanded = false;
        }
    }

    /// Detail panel under the table: full description + collapsible language list.
    fn draw_selected_model_details(&mut self, ui: &mut egui::Ui) {
        let Some(idx) = self.selected_service_idx else {
            return;
        };
        let Some(model) = self.services_info.get(idx) else {
            return;
        };

        // Snapshot fields so we don't hold a borrow of `self.services_info`
        // while toggling `services_langs_expanded` below.
        let display_name = model.display_name.clone();
        let description = model.description.clone();
        let reliability = model.reliability.clone();
        let speed = model.speed.clone();
        let price_per_1k = model.price_per_1000();
        let lang_count = model.languages_supported.len();
        let langs: Vec<String> = model
            .languages_supported
            .iter()
            .map(|l| l.language_name.clone())
            .collect();

        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("\u{25B6} {display_name}"))
                        .strong()
                        .size(14.0)
                        .color(egui::Color32::from_rgb(90, 142, 242)),
                );
                ui.label(
                    egui::RichText::new(format!(
                        "({reliability} \u{00B7} {speed} \u{00B7} ${price_per_1k:.4}/1k chars)"
                    ))
                    .small()
                    .color(egui::Color32::from_gray(160)),
                );
            });

            ui.add_space(4.0);
            ui.label(&description);

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("Languages ({lang_count}):"))
                        .small()
                        .color(egui::Color32::from_gray(180)),
                );
                let toggle_label = if self.services_langs_expanded { "Hide" } else { "Show all" };
                if ui.small_button(toggle_label).clicked() {
                    self.services_langs_expanded = !self.services_langs_expanded;
                }
            });

            if self.services_langs_expanded {
                ui.add_space(2.0);
                // Wrap language names into available width.
                ui.horizontal_wrapped(|ui| {
                    for name in &langs {
                        ui.label(
                            egui::RichText::new(name)
                                .small()
                                .color(egui::Color32::from_gray(190)),
                        );
                        ui.label(
                            egui::RichText::new("\u{00B7}")
                                .small()
                                .color(egui::Color32::from_gray(90)),
                        );
                    }
                });
            }
        });
    }
}

struct RowData {
    idx: usize,
    display_name: String,
    reliability: String,
    speed: String,
    price_per_1k: f64,
    lang_count: usize,
}

fn reliability_color(s: &str) -> egui::Color32 {
    match s {
        "high" => egui::Color32::from_rgb(120, 200, 120),
        "medium" => egui::Color32::from_rgb(255, 180, 80),
        "low" => egui::Color32::from_rgb(255, 120, 120),
        _ => egui::Color32::from_gray(180),
    }
}
