mod api;
mod config;
mod debug_log;
mod local_config;
mod se_contract;
mod ui;

use se_contract::{PluginSettings, read_request};

fn main() {
    debug_log::init_log();
    debug_log!("Plugin starting");

    let args: Vec<String> = std::env::args().collect();
    let request_path = args.get(1).expect("Usage: se-ai-translator <request.json path>");

    let se_request = match read_request(request_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error reading request: {e}");
            std::process::exit(1);
        }
    };

    // Load settings from SE5 request first, then merge with locally saved credentials
    let mut settings = PluginSettings::from_se_settings(se_request.settings.as_ref());
    
    // Try to load credentials from local config file
    match local_config::load_settings() {
        Ok(local_settings) => {
            // Merge: use local credentials if available
            if local_settings.has_credentials() {
                debug_log!("Using locally saved credentials");
                settings.auth_token = local_settings.auth_token;
                settings.username = local_settings.username;
                settings.password = local_settings.password;
            }
            // Always merge non-credential preferences
            if let Some(lang) = local_settings.last_source_lang {
                settings.last_source_lang = Some(lang);
            }
            if let Some(lang) = local_settings.last_target_lang {
                settings.last_target_lang = Some(lang);
            }
            if let Some(engine) = local_settings.last_engine {
                settings.last_engine = Some(engine);
            }
        }
        Err(e) => {
            debug_log!("Could not load local config: {e} (continuing with SE5 settings)");
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([420.0, 400.0])
            .with_min_inner_size([350.0, 300.0])
            .with_title("AI Translate (OpenSubtitles)"),
        ..Default::default()
    };

    let result = eframe::run_native(
        "SE5 AI Translator",
        options,
        Box::new(move |cc| {
            let mut app = ui::TranslatorApp::new(cc, settings);
            app.set_request(se_request);
            Ok(Box::new(app))
        }),
    );

    if let Err(e) = result {
        eprintln!("UI error: {e}");
        std::process::exit(1);
    }
}
