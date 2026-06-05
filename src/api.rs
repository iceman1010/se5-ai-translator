use crate::config::API_KEY;
use crate::debug_log;
use reqwest::blocking::multipart;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const API_BASE_URL: &str = "https://api.opensubtitles.com/api/v1";

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct LoginResponse {
    pub token: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TranslationStatusResponse {
    pub status: Option<String>,
    pub translation: Option<String>,
    pub correlation_id: Option<String>,
    pub errors: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageInfo {
    pub language_code: String,
    pub language_name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DetectedLanguage {
    #[serde(rename = "ISO_639_1")]
    pub iso_639_1: String,
    #[serde(rename = "W3C")]
    pub w3c: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct ApiClient {
    auth_token: String,
    client: reqwest::blocking::Client,
}

/// Top-level response shape for `POST /ai/credits`.
///
/// Actual API response (contrary to TS docs):
/// ```json
/// { "data": { "credits": 9098 } }
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct CreditBalanceResponse {
    pub data: CreditBalanceData,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreditBalanceData {
    /// Credits may come back as integer (`9098`) or fractional. `f64` accepts both.
    pub credits: f64,
}

/// One row from `POST /ai/credits/buy` (`data` array).
///
/// Note: contrary to the TS docs, `value` is the **price in USD** (e.g. `"5 USD"`),
/// not the credit count. The credit count is embedded in `name` (e.g. `"500 credits"`).
#[derive(Debug, Clone, Deserialize)]
pub struct CreditPackage {
    pub name: String,
    /// Price string, e.g. `"5 USD"`, `"10 USD"`.
    pub value: String,
    /// 0-100. Discount from regular price.
    pub discount_percent: f64,
    /// Browser-openable checkout URL.
    pub checkout_url: String,
}

impl CreditPackage {
    /// Best-effort extraction of the credit count from `name` (e.g. `"500 credits"` → `500`).
    pub fn credit_count(&self) -> Option<u64> {
        self.name
            .split_whitespace()
            .next()
            .and_then(|tok| tok.parse().ok())
    }
}

/// Top-level response shape for `POST /ai/credits/buy`.
///
/// Actual API response:
/// ```json
/// { "data": [ { name, value, discount_percent, checkout_url }, ... ] }
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct CreditPackagesResponse {
    pub data: Vec<CreditPackage>,
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

#[derive(Debug)]
pub enum TranslateError {
    Network(String),
    Auth(String),
    Api(String),
    Timeout,
    Cancelled,
}

impl std::fmt::Display for TranslateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(e) => write!(f, "Network error: {e}"),
            Self::Auth(e) => write!(f, "Authentication error: {e}"),
            Self::Api(e) => write!(f, "API error: {e}"),
            Self::Timeout => write!(f, "Translation timed out"),
            Self::Cancelled => write!(f, "Cancelled"),
        }
    }
}

impl ApiClient {
    pub fn new(auth_token: &str) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");
        Self {
            auth_token: auth_token.to_string(),
            client,
        }
    }

    pub fn login(username: &str, password: &str) -> Result<String, TranslateError> {
        let client = reqwest::blocking::Client::new();
        let resp = client
            .post(format!("{API_BASE_URL}/login"))
            .header("Api-Key", API_KEY)
            .header("Accept", "application/json")
            .header("User-Agent", "se-ai-translator v0.1.0")
            .json(&serde_json::json!({
                "username": username,
                "password": password
            }))
            .send()
            .map_err(|e| TranslateError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(TranslateError::Auth(format!("Login failed ({status}): {body}")));
        }

        let data: serde_json::Value = resp.json().map_err(|e| TranslateError::Auth(e.to_string()))?;
        data.get("token")
            .and_then(|t| t.as_str())
            .map(|t| t.to_string())
            .ok_or_else(|| TranslateError::Auth("No token in login response".to_string()))
    }

    fn common_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("Api-Key", API_KEY.parse().expect("invalid api key"));
        headers.insert(
            "Authorization",
            format!("Bearer {}", self.auth_token).parse().expect("invalid token"),
        );
        headers.insert("Accept", "application/json".parse().unwrap());
        headers.insert("User-Agent", "se-ai-translator v0.1.0".parse().unwrap());
        headers
    }

    pub fn detect_language(&self, srt_content: &str) -> Result<DetectedLanguage, TranslateError> {
        let tmp_dir = std::env::temp_dir().join("se-ai-translator-detect");
        std::fs::create_dir_all(&tmp_dir).map_err(|e| TranslateError::Api(e.to_string()))?;
        let file_path = tmp_dir.join("subtitle.srt");
        std::fs::write(&file_path, srt_content).map_err(|e| TranslateError::Api(e.to_string()))?;

        let file_part = multipart::Part::file(&file_path)
            .map_err(|e| TranslateError::Api(e.to_string()))?
            .file_name("subtitle.srt")
            .mime_str("text/plain")
            .map_err(|e| TranslateError::Api(e.to_string()))?;

        let form = multipart::Form::new().part("file", file_part);

        let resp = self
            .client
            .post(format!("{API_BASE_URL}/ai/detect_language"))
            .headers(self.common_headers())
            .multipart(form)
            .send()
            .map_err(|e| TranslateError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(TranslateError::Api(format!("Language detection failed ({status}): {body}")));
        }

        let data: serde_json::Value = resp.json().map_err(|e| TranslateError::Api(e.to_string()))?;

        let lang_data = data
            .get("data")
            .and_then(|d| d.get("language"))
            .cloned()
            .or_else(|| data.get("language").cloned())
            .ok_or_else(|| TranslateError::Api("No language in detection response".to_string()))?;

        let detected: DetectedLanguage =
            serde_json::from_value(lang_data).map_err(|e| TranslateError::Api(format!("Invalid detection response: {e}")))?;

        let _ = std::fs::remove_file(&file_path);
        Ok(detected)
    }

    pub fn fetch_engines(&self) -> Result<Vec<String>, TranslateError> {
        let resp = self
            .client
            .post(format!("{API_BASE_URL}/ai/info/translation_apis"))
            .headers(self.common_headers())
            .send()
            .map_err(|e| TranslateError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(TranslateError::Api(format!("Failed to fetch engines: {}", resp.status())));
        }

        let data: serde_json::Value = resp.json().map_err(|e| TranslateError::Api(e.to_string()))?;
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

    pub fn fetch_languages(&self, engine: Option<&str>) -> Result<Vec<LanguageInfo>, TranslateError> {
        let mut body = serde_json::Map::new();
        if let Some(e) = engine {
            body.insert("api".to_string(), serde_json::Value::String(e.to_string()));
        }

        let resp = self
            .client
            .post(format!("{API_BASE_URL}/ai/info/translation_languages"))
            .headers(self.common_headers())
            .json(&body)
            .send()
            .map_err(|e| TranslateError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(TranslateError::Api(format!("Failed to fetch languages: {}", resp.status())));
        }

        let data: serde_json::Value = resp.json().map_err(|e| TranslateError::Api(e.to_string()))?;
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
                            ) {
                                if !langs.iter().any(|l: &LanguageInfo| l.language_code == code) {
                                    langs.push(LanguageInfo {
                                        language_code: code.to_string(),
                                        language_name: name.to_string(),
                                    });
                                }
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

    pub fn translate(
        &self,
        srt_content: &str,
        source_lang: &str,
        target_lang: &str,
        engine: &str,
        cancel_flag: &std::sync::atomic::AtomicBool,
        progress_cb: &dyn Fn(f32),
    ) -> Result<String, TranslateError> {
        let tmp_dir = std::env::temp_dir().join("se-ai-translator-upload");
        std::fs::create_dir_all(&tmp_dir).map_err(|e| TranslateError::Api(e.to_string()))?;
        let file_path = tmp_dir.join("subtitle.srt");
        std::fs::write(&file_path, srt_content).map_err(|e| TranslateError::Api(e.to_string()))?;

        let file_part = multipart::Part::file(&file_path)
            .map_err(|e| TranslateError::Api(e.to_string()))?
            .file_name("subtitle.srt")
            .mime_str("text/plain")
            .map_err(|e| TranslateError::Api(e.to_string()))?;

        let effective_source = if source_lang == "auto" {
            "auto".to_string()
        } else {
            source_lang.to_string()
        };

        let form = multipart::Form::new()
            .part("file", file_part)
            .text("translate_from", effective_source.clone())
            .text("translate_to", target_lang.to_string())
            .text("api", engine.to_string())
            .text("return_content", "true".to_string());

        debug_log!("translate: from={effective_source} to={target_lang} engine={engine}");

        let resp = self
            .client
            .post(format!("{API_BASE_URL}/ai/translate"))
            .headers(self.common_headers())
            .multipart(form)
            .timeout(Duration::from_secs(120))
            .send()
            .map_err(|e| TranslateError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(TranslateError::Api(format!("Translation request failed ({status}): {body}")));
        }

        let data: serde_json::Value = resp.json().map_err(|e| TranslateError::Api(e.to_string()))?;
        debug_log!("translate response: {}", serde_json::to_string_pretty(&data).unwrap_or_default());

        let correlation_id = data
            .get("correlation_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                TranslateError::Api("No correlation_id in translation response".to_string())
            })?;

        if let Some(translation) = data.get("translation").and_then(|v| v.as_str()) {
            if !translation.is_empty() {
                let _ = std::fs::remove_file(&file_path);
                return Ok(translation.to_string());
            }
        }

        progress_cb(0.1);
        self.poll_translation(correlation_id, cancel_flag, progress_cb)
    }

    fn poll_translation(
        &self,
        correlation_id: &str,
        cancel_flag: &std::sync::atomic::AtomicBool,
        progress_cb: &dyn Fn(f32),
    ) -> Result<String, TranslateError> {
        let poll_client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create poll client");

        let max_attempts = 120u32;
        let mut attempt = 0u32;

        loop {
            if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(TranslateError::Cancelled);
            }

            attempt += 1;
            if attempt > max_attempts {
                return Err(TranslateError::Timeout);
            }

            std::thread::sleep(Duration::from_secs(3));

            let resp = poll_client
                .post(format!("{API_BASE_URL}/ai/translation/{correlation_id}"))
                .headers(self.common_headers())
                .send()
                .map_err(|e| TranslateError::Network(e.to_string()))?;

            if !resp.status().is_success() {
                continue;
            }

            let data: serde_json::Value = resp.json().map_err(|e| TranslateError::Api(e.to_string()))?;
            let status = data.get("status").and_then(|v| v.as_str()).unwrap_or("UNKNOWN");
            debug_log!("poll attempt={attempt} status={status} data={}", serde_json::to_string(&data).unwrap_or_default());

            match status {
                "COMPLETED" => {
                    let translation = data
                        .get("data")
                        .and_then(|d| d.get("return_content"))
                        .and_then(|v| v.as_str())
                        .or_else(|| {
                            data.get("data")
                                .and_then(|d| d.get("translation"))
                                .and_then(|v| v.as_str())
                        })
                        .or_else(|| data.get("translation").and_then(|v| v.as_str()))
                        .ok_or_else(|| {
                            debug_log!("poll COMPLETED but no content found. Full response: {}", serde_json::to_string(&data).unwrap_or_default());
                            TranslateError::Api("Translation completed but no content returned".to_string())
                        })?;

                    if translation.is_empty() {
                        return Err(TranslateError::Api("Translation returned empty content".to_string()));
                    }

                    progress_cb(1.0);
                    return Ok(translation.to_string());
                }
                "ERROR" | "TIMEOUT" => {
                    let errors = data
                        .get("errors")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    return Err(TranslateError::Api(if errors.is_empty() {
                        format!("Translation {status}")
                    } else {
                        format!("Translation {status}: {errors}")
                    }));
                }
                _ => {
                    let progress = 0.1 + (attempt as f32 / max_attempts as f32) * 0.8;
                    progress_cb(progress.min(0.9));
                }
            }
        }
    }

    /// Fetches the current credit balance for the logged-in user.
    ///
    /// Endpoint: `POST /ai/credits` with an empty JSON body.
    /// Returns the raw credit number (the API uses integer credits).
    pub fn get_credits(&self) -> Result<f64, TranslateError> {
        let resp = self
            .client
            .post(format!("{API_BASE_URL}/ai/credits"))
            .headers(self.common_headers())
            .header("Content-Type", "application/json")
            .json(&serde_json::Map::new())
            .send()
            .map_err(|e| TranslateError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(TranslateError::Api(format!(
                "Failed to fetch credits ({status}): {body}"
            )));
        }

        let data: CreditBalanceResponse = resp
            .json()
            .map_err(|e| TranslateError::Api(format!("Invalid credits response: {e}")))?;

        Ok(data.data.credits)
    }

    /// Fetches available credit purchase packages.
    ///
    /// Endpoint: `POST /ai/credits/buy`. Accepts either JSON or multipart form;
    /// we use multipart to match the TS client and to support an optional email field
    /// (which personalises the signed checkout URLs).
    pub fn get_credit_packages(&self, email: Option<&str>) -> Result<Vec<CreditPackage>, TranslateError> {
        let mut form = multipart::Form::new();
        if let Some(email) = email.filter(|s| !s.is_empty()) {
            form = form.text("email", email.to_string());
        }

        let resp = self
            .client
            .post(format!("{API_BASE_URL}/ai/credits/buy"))
            .headers(self.common_headers())
            .multipart(form)
            .send()
            .map_err(|e| TranslateError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(TranslateError::Api(format!(
                "Failed to fetch credit packages ({status}): {body}"
            )));
        }

        let data: CreditPackagesResponse = resp
            .json()
            .map_err(|e| TranslateError::Api(format!("Invalid credit packages response: {e}")))?;

        Ok(data.data)
    }

    /// Fetches the list of available AI services (translation + transcription
    /// models with pricing, reliability, speed, supported languages).
    ///
    /// Endpoint: `GET /ai/info/services`. Returns only the Translation models;
    /// transcription models are dropped at the API boundary since this plugin
    /// is translation-only.
    pub fn get_services_info(&self) -> Result<Vec<ServiceModel>, TranslateError> {
        let resp = self
            .client
            .get(format!("{API_BASE_URL}/ai/info/services"))
            .headers(self.common_headers())
            .send()
            .map_err(|e| TranslateError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(TranslateError::Api(format!(
                "Failed to fetch services info ({status}): {body}"
            )));
        }

        let data: ServicesInfoResponse = resp
            .json()
            .map_err(|e| TranslateError::Api(format!("Invalid services info response: {e}")))?;

        Ok(data.data.translation)
    }
}
