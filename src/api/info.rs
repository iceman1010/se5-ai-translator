//! Info endpoints: engines, languages, services catalog.

use super::{ApiClient, TranslateError, API_BASE_URL};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageInfo {
    pub language_code: String,
    pub language_name: String,
}

/// One model entry from `GET /ai/info/services` (`data.Translation[]`).
///
/// Note: API returns `pricing` as a coarse label (e.g. "Pay-per-character") and
/// `price` as the numeric per-character cost. `speed` is undocumented but always
/// present in practice ("slow" | "medium" | "fast" | "very slow").
#[derive(Debug, Clone, Deserialize)]
pub struct ServiceModel {
    /// Internal API identifier (e.g. "gemini3-flash"). Not currently displayed
    /// in the UI but kept for future use (e.g. deep-linking to a model).
    #[allow(dead_code)]
    pub name: String,
    pub display_name: String,
    pub description: String,
    /// Coarse pricing label, e.g. "Pay-per-character". Same for all translation
    /// models, so currently unused in the UI. Kept for completeness.
    #[allow(dead_code)]
    pub pricing: String,
    /// "low" | "medium" | "high"
    pub reliability: String,
    /// "slow" | "medium" | "fast" | "very slow"
    pub speed: String,
    /// Per-character cost.
    pub price: f64,
    pub languages_supported: Vec<LanguageInfo>,
}

impl ServiceModel {
    /// Per-1,000-character cost (raw `price` is per character and hard to compare).
    pub fn price_per_1000(&self) -> f64 {
        self.price * 1000.0
    }
}

/// Top-level response shape for `GET /ai/info/services`.
///
/// Actual API response:
/// ```json
/// { "data": { "Translation": [...], "Transcription": [...] } }
/// ```
///
/// Note: keys are PascalCase in the payload (not the snake_case the TS docs imply).
#[derive(Debug, Clone, Deserialize)]
pub struct ServicesInfoResponse {
    pub data: ServicesInfoData,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ServicesInfoData {
    #[serde(default)]
    pub translation: Vec<ServiceModel>,
    #[serde(default)]
    #[allow(dead_code)]
    pub transcription: Vec<serde_json::Value>,
}

impl ApiClient {
    #[allow(dead_code)]
    pub fn fetch_engines(&mut self) -> Result<Vec<String>, TranslateError> {
        let resp = self.send_authed("fetch_engines", |s| {
            s.client
                .post(format!("{API_BASE_URL}/ai/info/translation_apis"))
                .headers(s.common_headers())
        })?;

        let data: serde_json::Value = resp
            .json()
            .map_err(|e| TranslateError::Api(e.to_string()))?;
        let engines = data
            .get("data")
            .and_then(|d| d.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        Ok(engines)
    }

    pub fn fetch_languages(
        &mut self,
        engine: Option<&str>,
    ) -> Result<Vec<LanguageInfo>, TranslateError> {
        let mut body = serde_json::Map::new();
        if let Some(e) = engine {
            body.insert("api".to_string(), serde_json::Value::String(e.to_string()));
        }

        let resp = self.send_authed("fetch_languages", |s| {
            s.client
                .post(format!("{API_BASE_URL}/ai/info/translation_languages"))
                .headers(s.common_headers())
                .json(&body)
        })?;

        let data: serde_json::Value = resp
            .json()
            .map_err(|e| TranslateError::Api(e.to_string()))?;
        let data_obj = data.get("data").and_then(|d| d.as_object());

        let languages = if let Some(engine_name) = engine {
            data_obj
                .and_then(|obj| obj.get(engine_name))
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|lang| {
                            let code = lang.get("language_code").and_then(|v| v.as_str())?;
                            let name = lang.get("language_name").and_then(|v| v.as_str())?;
                            Some(LanguageInfo {
                                language_code: code.to_string(),
                                language_name: name.to_string(),
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        } else {
            let mut langs = Vec::new();
            if let Some(obj) = data_obj {
                for (_engine_name, lang_arr) in obj {
                    if let Some(arr) = lang_arr.as_array() {
                        for lang in arr {
                            if let (Some(code), Some(name)) = (
                                lang.get("language_code").and_then(|v| v.as_str()),
                                lang.get("language_name").and_then(|v| v.as_str()),
                            )
                                && !langs.iter().any(|l: &LanguageInfo| l.language_code == code) {
                                    langs.push(LanguageInfo {
                                        language_code: code.to_string(),
                                        language_name: name.to_string(),
                                    });
                                }
                        }
                    }
                }
            }
            langs
        };

        let mut languages = languages;
        languages.sort_by(|a, b| a.language_name.cmp(&b.language_name));
        Ok(languages)
    }

    /// Fetches the list of available AI services (translation + transcription
    /// models with pricing, reliability, speed, supported languages).
    ///
    /// Endpoint: `GET /ai/info/services`. Returns only the Translation models;
    /// transcription models are dropped at the API boundary since this plugin
    /// is translation-only.
    pub fn get_services_info(&mut self) -> Result<Vec<ServiceModel>, TranslateError> {
        let resp = self.send_authed("get_services_info", |s| {
            s.client
                .get(format!("{API_BASE_URL}/ai/info/services"))
                .headers(s.common_headers())
        })?;

        let data: ServicesInfoResponse = resp
            .json()
            .map_err(|e| TranslateError::Api(format!("Invalid services info response: {e}")))?;

        Ok(data.data.translation)
    }
}
