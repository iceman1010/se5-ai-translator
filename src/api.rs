use crate::config::API_KEY;
use crate::debug_log;
use reqwest::blocking::multipart;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
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
    /// Stored credentials so the client can silently re-authenticate when the
    /// server reports the current token as invalid/expired.
    username: Option<String>,
    password: Option<String>,
    /// If set, the client pushes transient human-readable status messages here
    /// (e.g. "Session expired, re-authenticating…") so the UI can surface them
    /// without coupling to the actual call sites.
    status_sink: Option<Arc<Mutex<Option<String>>>>,
    /// If the client refreshed the token during a call, the new value lands
    /// here so callers can persist it. Cleared by [`take_refreshed_token`].
    refreshed_token: Option<String>,
    client: reqwest::blocking::Client,
}

/// Shared type for the UI to receive transient status messages from the API
/// client (e.g. "Re-authenticating…"). None = idle/no message.
pub type StatusSink = Arc<Mutex<Option<String>>>;

impl ApiClient {
    /// Backwards-compatible constructor: no stored credentials, so no
    /// automatic re-authentication. Use [`ApiClient::with_credentials`] for
    /// callers that can supply username + password.
    #[allow(dead_code)]
    pub fn new(auth_token: &str) -> Self {
        Self::build(auth_token, None, None, None)
    }

    /// Constructor that enables automatic re-authentication on token expiry.
    /// `status_sink` is optional — pass `None` if the caller doesn't care to
    /// surface "Re-authenticating…" status messages.
    pub fn with_credentials(
        auth_token: &str,
        username: Option<String>,
        password: Option<String>,
        status_sink: Option<StatusSink>,
    ) -> Self {
        Self::build(auth_token, username, password, status_sink)
    }

    fn build(
        auth_token: &str,
        username: Option<String>,
        password: Option<String>,
        status_sink: Option<StatusSink>,
    ) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");
        Self {
            auth_token: auth_token.to_string(),
            username,
            password,
            status_sink,
            refreshed_token: None,
            client,
        }
    }

    /// Returns whether this client is capable of refreshing its own token
    /// (i.e. credentials are present). Used by [`send_authed`] to decide
    /// whether a 401 is recoverable.
    fn has_credentials(&self) -> bool {
        self.username.as_ref().is_some_and(|u| !u.is_empty())
            && self.password.as_ref().is_some_and(|p| !p.is_empty())
    }

    /// Push a transient status message to the UI sink (if any).
    fn push_status(&self, msg: Option<&str>) {
        if let Some(sink) = &self.status_sink {
            if let Ok(mut g) = sink.lock() {
                *g = msg.map(String::from);
            }
        }
    }

    /// If the client refreshed the token during a call, take ownership of the
    /// new value. Subsequent calls return `None`. The UI uses this to persist
    /// the refreshed token back into plugin settings.
    pub fn take_refreshed_token(&mut self) -> Option<String> {
        self.refreshed_token.take()
    }

    /// Re-authenticate using stored credentials. Updates `self.auth_token` and
    /// records the new value in `self.refreshed_token` for later persistence.
    fn refresh_token(&mut self) -> Result<(), TranslateError> {
        let (Some(u), Some(p)) = (self.username.clone(), self.password.clone()) else {
            return Err(TranslateError::Auth(
                "Cannot refresh token: no stored credentials".to_string(),
            ));
        };
        self.push_status(Some("Session expired, re-authenticating…"));
        let new_token = Self::login(&u, &p)?;
        self.auth_token = new_token.clone();
        self.refreshed_token = Some(new_token);
        self.push_status(None);
        Ok(())
    }

    /// Decide whether a failed response body indicates an auth failure that
    /// [`refresh_token`] could plausibly fix. Looks at the response body
    /// content, not just the HTTP status, because some auth-protected
    /// endpoints return non-401 statuses with an auth-related error in the
    /// body, and we want to be defensive against future API changes.
    ///
    /// Known auth-failure body (from probing the live API):
    /// ```json
    /// {"error":"Please login again, your authorization token is invalid, ...",
    ///  "STATUS":"ERROR"}
    /// ```
    fn is_auth_failure(status: reqwest::StatusCode, body: &str) -> bool {
        let auth_phrase =
            body.contains("authorization token is invalid") || body.contains("Please login again");
        // Strong signal: STATUS=ERROR + auth-specific text. This is what the
        // server returns today for expired/invalid/missing tokens.
        if auth_phrase && body.contains("\"STATUS\":\"ERROR\"") {
            return true;
        }
        // Backup signal: HTTP 401 + the same auth phrase. Catches the case
        // where the server ever drops the STATUS field.
        if status == reqwest::StatusCode::UNAUTHORIZED && auth_phrase {
            return true;
        }
        false
    }

    /// Send an authenticated request, retrying once with a freshly issued
    /// token if the server reports the current token as invalid.
    ///
    /// `op_name` is used purely for logging (e.g. `"get_credits"`).
    /// `build_request` is called at most twice — once with the cached token
    /// and, on a recoverable auth failure, again after `refresh_token`.
    fn send_authed(
        &mut self,
        op_name: &str,
        build_request: impl Fn(&Self) -> reqwest::blocking::RequestBuilder,
    ) -> Result<reqwest::blocking::Response, TranslateError> {
        for attempt in 1..=2u8 {
            let resp = build_request(self)
                .send()
                .map_err(|e| TranslateError::Network(format!("{op_name}: {e}")))?;
            let status = resp.status();

            if status.is_success() {
                debug_log!("{op_name}: HTTP {status} OK");
                return Ok(resp);
            }

            // Failed. Read body for diagnosis + log it.
            let body = resp.text().unwrap_or_default();
            debug_log!("{op_name}: attempt={attempt} HTTP {status} body={body}");

            if attempt == 1 && self.has_credentials() && Self::is_auth_failure(status, &body) {
                debug_log!("{op_name}: auth failure detected, refreshing token");
                if let Err(e) = self.refresh_token() {
                    debug_log!("{op_name}: refresh_token failed: {e}");
                    return Err(TranslateError::Auth(format!(
                        "{op_name}: token expired and re-login failed: {e}"
                    )));
                }
                debug_log!("{op_name}: token refreshed, retrying request");
                continue;
            }

            return Err(TranslateError::Api(format!(
                "{op_name} failed ({status}): {body}"
            )));
        }
        // Loop runs at most twice; we always return inside the loop.
        unreachable!("send_authed loop exhausted without returning")
    }
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

/// Pricing / usage info returned by the API on a completed translation.
///
/// All fields live under `.data` in the COMPLETED poll response:
/// ```json
/// { "data": { "characters_count": 1616, "unit_price": 0.00072,
///             "total_price": 2, "credits_left": 9092, "duration": 11 } }
/// ```
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct TranslationUsage {
    /// Number of characters charged for this translation.
    pub characters_count: u64,
    /// Per-character price (USD-style fractional value).
    pub unit_price: f64,
    /// Total credits charged for this translation.
    pub total_price: f64,
    /// Remaining credit balance after this translation.
    pub credits_left: f64,
    /// Server-reported translation duration in seconds.
    pub duration: f64,
}

/// Successful translation payload returned by [`ApiClient::translate`].
#[derive(Debug, Clone)]
pub struct TranslationResult {
    pub translation: String,
    pub usage: Option<TranslationUsage>,
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
    pub fn login(username: &str, password: &str) -> Result<String, TranslateError> {
        debug_log!("login: attempting login for user={username}");
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
            debug_log!("login: HTTP {status} body={body}");
            return Err(TranslateError::Auth(format!(
                "Login failed ({status}): {body}"
            )));
        }

        let data: serde_json::Value = resp
            .json()
            .map_err(|e| TranslateError::Auth(e.to_string()))?;
        debug_log!("login: HTTP 200 OK");
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
            format!("Bearer {}", self.auth_token)
                .parse()
                .expect("invalid token"),
        );
        headers.insert("Accept", "application/json".parse().unwrap());
        headers.insert("User-Agent", "se-ai-translator v0.1.0".parse().unwrap());
        headers
    }

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
        &mut self,
        srt_content: &str,
        source_lang: &str,
        target_lang: &str,
        engine: &str,
        cancel_flag: &std::sync::atomic::AtomicBool,
        progress_cb: &dyn Fn(f32),
    ) -> Result<TranslationResult, TranslateError> {
        // Read bytes up front so the multipart form can be re-built inside
        // the closure on retry without re-reading from disk.
        let tmp_dir = std::env::temp_dir().join("se-ai-translator-upload");
        std::fs::create_dir_all(&tmp_dir).map_err(|e| TranslateError::Api(e.to_string()))?;
        let file_path = tmp_dir.join("subtitle.srt");
        std::fs::write(&file_path, srt_content).map_err(|e| TranslateError::Api(e.to_string()))?;
        let srt_bytes = std::fs::read(&file_path)
            .map_err(|e| TranslateError::Api(format!("failed to read temp file: {e}")))?;

        let effective_source = if source_lang == "auto" {
            "auto".to_string()
        } else {
            source_lang.to_string()
        };

        debug_log!("translate: from={effective_source} to={target_lang} engine={engine}");

        let resp = self.send_authed("translate", |s| {
            let part = multipart::Part::bytes(srt_bytes.clone())
                .file_name("subtitle.srt")
                .mime_str("text/plain")
                .expect("static mime string");
            let form = multipart::Form::new()
                .part("file", part)
                .text("translate_from", effective_source.clone())
                .text("translate_to", target_lang.to_string())
                .text("api", engine.to_string())
                .text("return_content", "true".to_string());
            s.client
                .post(format!("{API_BASE_URL}/ai/translate"))
                .headers(s.common_headers())
                .multipart(form)
                .timeout(Duration::from_secs(120))
        })?;

        let _ = std::fs::remove_file(&file_path);

        let data: serde_json::Value = resp
            .json()
            .map_err(|e| TranslateError::Api(e.to_string()))?;
        debug_log!(
            "translate response: {}",
            serde_json::to_string_pretty(&data).unwrap_or_default()
        );

        let correlation_id = data
            .get("correlation_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                TranslateError::Api("No correlation_id in translation response".to_string())
            })?;

        if let Some(translation) = data.get("translation").and_then(|v| v.as_str()) {
            if !translation.is_empty() {
                return Ok(TranslationResult {
                    translation: translation.to_string(),
                    usage: parse_usage_from_data(&data),
                });
            }
        }

        progress_cb(0.1);
        self.poll_translation(correlation_id, cancel_flag, progress_cb)
    }

    fn poll_translation(
        &mut self,
        correlation_id: &str,
        cancel_flag: &std::sync::atomic::AtomicBool,
        progress_cb: &dyn Fn(f32),
    ) -> Result<TranslationResult, TranslateError> {
        let max_attempts = 120u32;
        let mut attempt = 0u32;
        let mut auth_retries = 0u8;

        loop {
            if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(TranslateError::Cancelled);
            }

            attempt += 1;
            if attempt > max_attempts {
                return Err(TranslateError::Timeout);
            }

            std::thread::sleep(Duration::from_secs(3));

            let resp = self
                .client
                .post(format!("{API_BASE_URL}/ai/translation/{correlation_id}"))
                .headers(self.common_headers())
                .send()
                .map_err(|e| TranslateError::Network(e.to_string()))?;

            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().unwrap_or_default();
                debug_log!(
                    "poll attempt={attempt} HTTP {status} body={body} (auth_retries={auth_retries})"
                );
                if auth_retries == 0
                    && self.has_credentials()
                    && Self::is_auth_failure(status, &body)
                {
                    debug_log!("poll: auth failure, refreshing token");
                    auth_retries += 1;
                    self.refresh_token()?;
                    // Don't increment attempt — we want to retry the same poll
                    // immediately without burning an attempt slot.
                    attempt -= 1;
                    continue;
                }
                // Non-recoverable HTTP failure: try again next iteration.
                continue;
            }

            let data: serde_json::Value = resp
                .json()
                .map_err(|e| TranslateError::Api(e.to_string()))?;
            let poll_status = data
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("UNKNOWN");
            debug_log!(
                "poll attempt={attempt} status={poll_status} data={}",
                serde_json::to_string(&data).unwrap_or_default()
            );

            match poll_status {
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
                            debug_log!(
                                "poll COMPLETED but no content found. Full response: {}",
                                serde_json::to_string(&data).unwrap_or_default()
                            );
                            TranslateError::Api(
                                "Translation completed but no content returned".to_string(),
                            )
                        })?;

                    if translation.is_empty() {
                        return Err(TranslateError::Api(
                            "Translation returned empty content".to_string(),
                        ));
                    }

                    progress_cb(1.0);
                    let usage = parse_usage_from_data(&data);
                    return Ok(TranslationResult {
                        translation: translation.to_string(),
                        usage,
                    });
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
                        format!("Translation {poll_status}")
                    } else {
                        format!("Translation {poll_status}: {errors}")
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
    pub fn get_credits(&mut self) -> Result<f64, TranslateError> {
        let resp = self.send_authed("get_credits", |s| {
            s.client
                .post(format!("{API_BASE_URL}/ai/credits"))
                .headers(s.common_headers())
                .header("Content-Type", "application/json")
                .json(&serde_json::Map::new())
        })?;

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
    pub fn get_credit_packages(
        &mut self,
        email: Option<&str>,
    ) -> Result<Vec<CreditPackage>, TranslateError> {
        // Email may be None / empty — store an Option<String> so the closure
        // can rebuild the form on retry without re-checking emptiness.
        let email_owned: Option<String> = email
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let resp = self.send_authed("get_credit_packages", |s| {
            let mut form = multipart::Form::new();
            if let Some(e) = &email_owned {
                form = form.text("email", e.clone());
            }
            s.client
                .post(format!("{API_BASE_URL}/ai/credits/buy"))
                .headers(s.common_headers())
                .multipart(form)
        })?;

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

/// Extracts usage / pricing fields from a translation response.
///
/// Accepts both inline (`data` is the response root) and polled
/// (`data` is nested under `.data`) shapes. Returns `None` only when no
/// pricing fields are present at all — which happens on inline-fast-path
/// submissions that did not actually bill yet.
fn parse_usage_from_data(resp: &serde_json::Value) -> Option<TranslationUsage> {
    let data = resp.get("data").unwrap_or(resp);

    let credits_left = data.get("credits_left").and_then(|v| v.as_f64());
    let total_price = data.get("total_price").and_then(|v| v.as_f64());

    // Require at least one of the two most meaningful fields, otherwise the
    // response simply doesn't carry pricing info (e.g. inline-fast-path).
    if credits_left.is_none() && total_price.is_none() {
        return None;
    }

    Some(TranslationUsage {
        characters_count: data
            .get("characters_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        unit_price: data.get("unit_price").and_then(|v| v.as_f64()).unwrap_or(0.0),
        total_price: total_price.unwrap_or(0.0),
        credits_left: credits_left.unwrap_or(0.0),
        duration: data.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0),
    })
}
