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

#[derive(Debug, Clone)]
pub struct ApiClient {
    api_key: String,
    auth_token: String,
    client: reqwest::blocking::Client,
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
    pub fn new(api_key: &str, auth_token: &str) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");
        Self {
            api_key: api_key.to_string(),
            auth_token: auth_token.to_string(),
            client,
        }
    }

    pub fn login(api_key: &str, username: &str, password: &str) -> Result<String, TranslateError> {
        let client = reqwest::blocking::Client::new();
        let resp = client
            .post(format!("{API_BASE_URL}/login"))
            .header("Api-Key", api_key)
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
        headers.insert("Api-Key", self.api_key.parse().expect("invalid api key"));
        headers.insert(
            "Authorization",
            format!("Bearer {}", self.auth_token).parse().expect("invalid token"),
        );
        headers.insert("Accept", "application/json".parse().unwrap());
        headers.insert("User-Agent", "se-ai-translator v0.1.0".parse().unwrap());
        headers
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
        let mut languages = Vec::new();
        if let Some(data_obj) = data.get("data").and_then(|d| d.as_object()) {
            for (_engine_name, langs) in data_obj {
                if let Some(lang_arr) = langs.as_array() {
                    for lang in lang_arr {
                        if let (Some(code), Some(name)) = (
                            lang.get("language_code").and_then(|v| v.as_str()),
                            lang.get("language_name").and_then(|v| v.as_str()),
                        ) {
                            if !languages.iter().any(|l: &LanguageInfo| l.language_code == code) {
                                languages.push(LanguageInfo {
                                    language_code: code.to_string(),
                                    language_name: name.to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }
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
            .text("translate_from", effective_source)
            .text("translate_to", target_lang.to_string())
            .text("api", engine.to_string())
            .text("return_content", "true".to_string());

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

            match status {
                "COMPLETED" => {
                    let translation = data
                        .get("translation")
                        .and_then(|v| v.as_str())
                        .or_else(|| {
                            data.get("data")
                                .and_then(|d| d.get("translation"))
                                .and_then(|v| v.as_str())
                        })
                        .ok_or_else(|| {
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
}
