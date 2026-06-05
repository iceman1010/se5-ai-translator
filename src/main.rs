mod api;
mod config;
mod debug_log;
mod se_contract;
mod ui;

use se_contract::{PluginSettings, read_request};

fn load_icon() -> Option<egui::IconData> {
    let img = image::load_from_memory(include_bytes!("../icon.png")).ok()?;
    let rgba = img.into_rgba8();
    let (w, h) = rgba.dimensions();
    Some(egui::IconData {
        rgba: rgba.into_raw(),
        width: w,
        height: h,
    })
}

fn main() {
    debug_log::init_log();
    debug_log!("Plugin starting");

    let args: Vec<String> = std::env::args().collect();
    let request_path = args
        .get(1)
        .expect("Usage: se-ai-translator <request.json path>");

    let se_request = match read_request(request_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error reading request: {e}");
            std::process::exit(1);
        }
    };

    let settings = PluginSettings::from_se_settings(se_request.settings.as_ref());

    let icon = load_icon();
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([420.0, 400.0])
        .with_min_inner_size([350.0, 300.0])
        .with_title("AI Translate (OpenSubtitles)");
    let viewport = if let Some(icon) = icon {
        viewport.with_icon(icon)
    } else {
        viewport
    };
    let options = eframe::NativeOptions {
        viewport,
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
