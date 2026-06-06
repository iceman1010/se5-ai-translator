//! Language detection endpoint.

use super::{ApiClient, TranslateError, API_BASE_URL};
use reqwest::blocking::multipart;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct DetectedLanguage {
    #[serde(rename = "ISO_639_1")]
    pub iso_639_1: String,
    #[serde(rename = "W3C")]
    pub w3c: String,
    pub name: String,
}

impl ApiClient {
    pub fn detect_language(&mut self, srt_content: &str) -> Result<DetectedLanguage, TranslateError> {
        // Read the file bytes up front so we can re-construct the multipart
        // form on a retry without re-reading from disk. Part::bytes is also
        // easier to clone than Part::file (which captures a Path).
        let tmp_dir = std::env::temp_dir().join("se-ai-translator-detect");
        std::fs::create_dir_all(&tmp_dir).map_err(|e| TranslateError::Api(e.to_string()))?;
        let file_path = tmp_dir.join("subtitle.srt");
        std::fs::write(&file_path, srt_content).map_err(|e| TranslateError::Api(e.to_string()))?;
        let srt_bytes = std::fs::read(&file_path)
            .map_err(|e| TranslateError::Api(format!("failed to read temp file: {e}")))?;

        let resp = self.send_authed("detect_language", |s| {
            let part = multipart::Part::bytes(srt_bytes.clone())
                .file_name("subtitle.srt")
                .mime_str("text/plain")
                .expect("static mime string");
            let form = multipart::Form::new().part("file", part);
            s.client
                .post(format!("{API_BASE_URL}/ai/detect_language"))
                .headers(s.common_headers())
                .multipart(form)
        })?;

        let data: serde_json::Value = resp
            .json()
            .map_err(|e| TranslateError::Api(e.to_string()))?;

        let lang_data = data
            .get("data")
            .and_then(|d| d.get("language"))
            .cloned()
            .or_else(|| data.get("language").cloned())
            .ok_or_else(|| TranslateError::Api("No language in detection response".to_string()))?;

        let detected: DetectedLanguage = serde_json::from_value(lang_data)
            .map_err(|e| TranslateError::Api(format!("Invalid detection response: {e}")))?;

        let _ = std::fs::remove_file(&file_path);
        Ok(detected)
    }
}
