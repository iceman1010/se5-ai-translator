use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct SeRequest {
    pub api_version: u32,
    pub request_type: String,
    pub response_file_path: String,
    #[allow(dead_code)]
    pub temp_directory: Option<String>,
    pub subtitle: SeSubtitle,
    #[allow(dead_code)]
    pub selected_indices: Vec<usize>,
    #[allow(dead_code)]
    pub video_file_name: Option<String>,
    #[allow(dead_code)]
    pub frame_rate: Option<f64>,
    #[allow(dead_code)]
    pub video_duration_seconds: Option<f64>,
    #[allow(dead_code)]
    pub video_width: Option<u32>,
    #[allow(dead_code)]
    pub video_height: Option<u32>,
    #[allow(dead_code)]
    pub ui_language: Option<String>,
    #[allow(dead_code)]
    pub theme: Option<String>,
    #[allow(dead_code)]
    pub theme_colors: Option<SeThemeColors>,
    #[allow(dead_code)]
    pub se_version: Option<String>,
    pub settings: Option<serde_json::Value>,
    pub settings_version: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeSubtitle {
    #[allow(dead_code)]
    pub format: Option<String>,
    #[allow(dead_code)]
    pub file_name: Option<String>,
    #[allow(dead_code)]
    pub native: Option<String>,
    pub sub_rip: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct SeThemeColors {
    pub is_dark: Option<bool>,
    pub background_color: Option<String>,
    pub foreground_color: Option<String>,
    pub accent_color: Option<String>,
    pub background_color_lighter: Option<String>,
    pub background_color_header: Option<String>,
    pub bookmark_color: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeResponse {
    pub api_version: u32,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<SeResponseSubtitle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo_description: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeResponseSubtitle {
    pub format: String,
    pub native: String,
}

impl SeResponse {
    pub fn ok(translated_srt: &str, settings: &PluginSettings) -> Self {
        Self {
            api_version: 1,
            status: "ok".to_string(),
            message: None,
            subtitle: Some(SeResponseSubtitle {
                format: "SubRip".to_string(),
                native: translated_srt.to_string(),
            }),
            settings: serde_json::to_value(settings).ok(),
            settings_version: Some(2),
            undo_description: Some("AI Translate (OpenSubtitles)".to_string()),
        }
    }

    pub fn cancelled(settings: &PluginSettings) -> Self {
        Self {
            api_version: 1,
            status: "cancelled".to_string(),
            message: None,
            subtitle: None,
            settings: serde_json::to_value(settings).ok(),
            settings_version: Some(2),
            undo_description: None,
        }
    }

    pub fn error(msg: &str, settings: &PluginSettings) -> Self {
        Self {
            api_version: 1,
            status: "error".to_string(),
            message: Some(msg.to_string()),
            subtitle: None,
            settings: serde_json::to_value(settings).ok(),
            settings_version: Some(2),
            undo_description: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_source_lang: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_target_lang: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_engine: Option<String>,
}

impl Default for PluginSettings {
    fn default() -> Self {
        Self {
            auth_token: None,
            username: None,
            password: None,
            last_source_lang: Some("auto".to_string()),
            last_target_lang: None,
            last_engine: None,
        }
    }
}

impl PluginSettings {
    pub fn from_se_settings(settings: Option<&serde_json::Value>) -> Self {
        settings
            .and_then(|v| serde_json::from_value::<PluginSettings>(v.clone()).ok())
            .unwrap_or_default()
    }

    pub fn has_credentials(&self) -> bool {
        self.auth_token.as_ref().is_some_and(|t| !t.is_empty())
    }
}

pub fn read_request(path: &str) -> Result<SeRequest, String> {
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("Cannot read request file: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("Invalid request JSON: {e}"))
}

pub fn write_response(response: &SeResponse, path: &str) -> Result<(), String> {
    let json = serde_json::to_string_pretty(response)
        .map_err(|e| format!("Cannot serialize response: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("Cannot write response file: {e}"))
}
