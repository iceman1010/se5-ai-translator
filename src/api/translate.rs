//! Translate + poll endpoints, usage/cost parsing.

use super::{ApiClient, TranslateError, API_BASE_URL};
use crate::debug_log;
use reqwest::blocking::multipart;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TranslationStatusResponse {
    pub status: Option<String>,
    pub translation: Option<String>,
    pub correlation_id: Option<String>,
    pub errors: Option<Vec<String>>,
}

/// Pricing / usage info returned by the API on a completed translation.
///
/// All fields live under `.data` in the COMPLETED poll response:
/// ```json
/// { "data": { "characters_count": 1616, "unit_price": 0.00072,
///             "total_price": 2, "credits_left": 9092, "duration": 11 } }
/// ```
#[derive(Debug, Clone, Default, Serialize)]
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

impl ApiClient {
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

        if let Some(translation) = data.get("translation").and_then(|v| v.as_str())
            && !translation.is_empty() {
                return Ok(TranslationResult {
                    translation: translation.to_string(),
                    usage: parse_usage_from_data(&data),
                });
            }

        progress_cb(0.1);
        self.poll_translation(correlation_id, cancel_flag, progress_cb)
    }

    pub(crate) fn poll_translation(
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
