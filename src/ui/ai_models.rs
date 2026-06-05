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
                    debug_log!(
                        "refresh_services_info: WARNING — empty translation list, possible serde mismatch"
                    );
                }
                self.services_info = models;
                // Keep current selection if still valid. Don't auto-select: an
                // unselected state reads cleaner (no surprise blue row).
                if let Some(idx) = self.selected_service_idx {
                    if idx >= self.services_info.len() {
                        self.selected_service_idx = None;
                    }
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
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new("All costs are in credits per 1,000 characters.")
                        .small()
                        .color(egui::Color32::from_gray(150)),
                );
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
                            egui::RichText::new("No models available.").color(egui::Color32::GRAY),
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

                if matches!(self.services_state, CreditsLoadState::Loaded)
                    && !self.services_info.is_empty()
                {
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

    /// Modal showing all languages for a specific model, with a search filter.
    /// Triggered by clicking the info button next to a model's language count
    /// in the table. Closes when the user clicks Close or presses Escape.
    pub fn draw_services_langs_modal(&mut self, ctx: &egui::Context) {
        let Some(idx) = self.services_langs_modal_idx else {
            return;
        };

        // Snapshot model data up-front. We can't hold a borrow of
        // `self.services_info` while the window closure borrows `self` to
        // mutate `services_langs_search`.
        let Some(model) = self.services_info.get(idx) else {
            // Model list changed (e.g. refreshed); close the modal.
            self.services_langs_modal_idx = None;
            self.services_langs_search.clear();
            return;
        };
        let display_name = model.display_name.clone();
        let total_count = model.languages_supported.len();
        let all_langs: Vec<(String, String)> = model
            .languages_supported
            .iter()
            .map(|l| (l.language_code.clone(), l.language_name.clone()))
            .collect();

        // NOTE: no `paint_backdrop` here. The backdrop is `Order::Foreground`
        // which renders on top of the window in some configurations and dims
        // the entire modal surface. The update dialog gets away with it because
        // it's small; this modal is larger and the dim shows. The window's own
        // shadow + frame is enough emphasis for a non-blocking language list.

        let mut do_close = false;

        egui::Window::new(format!("Languages for {display_name}"))
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(
                egui::Frame::window(&ui_style_for_modal(ctx))
                    .corner_radius(10.0)
                    // Explicit fill so the window never blends with whatever
                    // is drawn behind it.
                    .fill(egui::Color32::from_rgb(30, 32, 42))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 4],
                        blur: 24,
                        spread: 4,
                        color: egui::Color32::from_black_alpha(160),
                    }),
            )
            .show(ctx, |ui| {
                ui.set_width(420.0);

                // Custom title bar: model name on the left, circular ✕ on the
                // right. The default egui title bar has no close affordance, so
                // we drop it (`title_bar(false)` above) and draw our own.
                ui.horizontal(|ui| {
                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new(format!("Languages for {display_name}"))
                            .strong()
                            .color(egui::Color32::from_gray(230)),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // 22px square button with 11px rounding == perfect circle.
                        let btn = egui::Button::new(
                            egui::RichText::new("\u{2715}")
                                .color(egui::Color32::from_gray(220))
                                .size(12.0),
                        )
                        .fill(egui::Color32::from_rgb(70, 75, 90))
                        .stroke(egui::Stroke::new(
                            1.0,
                            egui::Color32::from_rgb(110, 115, 130),
                        ))
                        .min_size(egui::Vec2::splat(22.0))
                        .corner_radius(egui::CornerRadius::same(11));

                        if ui.add(btn).clicked() {
                            do_close = true;
                        }
                    });
                });

                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                ui.label(
                    egui::RichText::new(format!("{total_count} supported languages"))
                        .small()
                        .color(egui::Color32::from_gray(170)),
                );
                ui.add_space(6.0);

                // Search field.
                ui.horizontal(|ui| {
                    ui.label("Search:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.services_langs_search)
                            .hint_text("Type to filter...")
                            .desired_width(220.0),
                    );
                    if ui.small_button("Clear").clicked() {
                        self.services_langs_search.clear();
                    }
                });
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(2.0);

                // Filter + scrollable list.
                let needle = self.services_langs_search.to_lowercase();
                let filtered: Vec<&(String, String)> = all_langs
                    .iter()
                    .filter(|(code, name)| {
                        if needle.is_empty() {
                            true
                        } else {
                            name.to_lowercase().contains(&needle)
                                || code.to_lowercase().contains(&needle)
                        }
                    })
                    .collect();

                let shown = filtered.len();
                ui.label(
                    egui::RichText::new(format!(
                        "{} of {} {}",
                        shown,
                        total_count,
                        if shown == 1 { "match" } else { "matches" }
                    ))
                    .small()
                    .color(egui::Color32::from_gray(160)),
                );
                ui.add_space(4.0);

                // Scrollable list with a fixed visible height. Without this,
                // the window would try to grow to fit all languages and either
                // overflow the screen or push the Close button off-screen.
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for (code, name) in &filtered {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(code)
                                        .monospace()
                                        .color(egui::Color32::from_rgb(120, 160, 220))
                                        .small(),
                                );
                                ui.label(
                                    egui::RichText::new(name).color(egui::Color32::from_gray(220)),
                                );
                            });
                        }
                        if filtered.is_empty() {
                            ui.label(
                                egui::RichText::new("No languages match your search.")
                                    .color(egui::Color32::from_gray(150))
                                    .italics(),
                            );
                        }
                    });
            });

        // Escape also closes the modal.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            do_close = true;
        }

        if do_close {
            self.services_langs_modal_idx = None;
            self.services_langs_search.clear();
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
        let mut open_langs_modal_for: Option<usize> = None;

        let table = TableBuilder::new(ui)
            .striped(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .min_scrolled_height(0.0)
            .max_scroll_height(f32::INFINITY)
            .column(Column::remainder().at_least(160.0))
            .column(Column::auto().at_least(80.0))
            .column(Column::auto().at_least(80.0))
            .column(Column::auto().at_least(90.0))
            // Wider than before to fit "57 [ℹ]" without truncation.
            .column(Column::auto().at_least(110.0));

        table
            .header(24.0, |mut header| {
                header.col(|ui| {
                    ui.strong("Model");
                });
                header.col(|ui| {
                    ui.strong("Reliability");
                });
                header.col(|ui| {
                    ui.strong("Speed");
                });
                header.col(|ui| {
                    ui.strong("Credits / 1k");
                });
                header.col(|ui| {
                    ui.strong("Languages");
                });
            })
            .body(|mut body| {
                let row_h = 22.0_f32;
                for row_data in &rows {
                    let is_selected = selected == Some(row_data.idx);
                    let mut clicked_this_row = false;
                    body.row(row_h, |mut row| {
                        // Model name
                        row.col(|ui| {
                            let text = egui::RichText::new(&row_data.display_name).strong().color(
                                if is_selected {
                                    egui::Color32::from_rgb(90, 142, 242)
                                } else {
                                    egui::Color32::from_gray(220)
                                },
                            );
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

                        // Credits per 1k chars (no currency symbol — the API
                        // returns credit costs, not dollars).
                        row.col(|ui| {
                            ui.label(format!("{:.4}", row_data.price_per_1k));
                        });

                        // Language count + info button
                        row.col(|ui| {
                            ui.label(format!("{}", row_data.lang_count));
                            // Small info button. Uses a unique id per row so egui
                            // can track click state independently.
                            let info_btn = ui.small_button("\u{2139}");
                            if info_btn.clicked() {
                                open_langs_modal_for = Some(row_data.idx);
                            }
                            info_btn.on_hover_text("Show supported languages");
                        });

                        // Click anywhere on the row (other than the info button)
                        // to select it.
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
        }
        if let Some(idx) = open_langs_modal_for {
            self.services_langs_modal_idx = Some(idx);
            self.services_langs_search.clear();
        }
    }

    /// Detail panel under the table: full description of the selected model.
    /// (Languages are now viewed via the modal triggered by the info button in
    /// the table, so they're no longer duplicated here.)
    fn draw_selected_model_details(&mut self, ui: &mut egui::Ui) {
        let Some(idx) = self.selected_service_idx else {
            return;
        };
        let Some(model) = self.services_info.get(idx) else {
            return;
        };

        // Snapshot fields up-front so we don't hold a borrow of
        // `self.services_info` while the closure builds UI.
        let display_name = model.display_name.clone();
        let description = model.description.clone();
        let reliability = model.reliability.clone();
        let speed = model.speed.clone();
        let price_per_1k = model.price_per_1000();
        let lang_count = model.languages_supported.len();

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
                        "({reliability} \u{00B7} {speed} \u{00B7} {price_per_1k:.4} credits/1k chars \u{00B7} {lang_count} languages)"
                    ))
                    .small()
                    .color(egui::Color32::from_gray(160)),
                );
            });

            ui.add_space(4.0);
            ui.label(&description);
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
