//! OpenSubtitles AI API client.
//!
//! Submodules:
//! - [`auth`] — login, token refresh, auth-failure detection
//! - [`translate`] — translate + poll endpoints, usage/cost parsing
//! - [`detect`] — language detection
//! - [`info`] — engines, languages, services catalog
//! - [`credits`] — balance + purchase packages

mod auth;
mod credits;
mod detect;
mod info;
mod translate;

#[allow(unused_imports)]
pub use auth::LoginResponse;
#[allow(unused_imports)]
pub use credits::{CreditBalanceData, CreditBalanceResponse, CreditPackage, CreditPackagesResponse};
#[allow(unused_imports)]
pub use detect::DetectedLanguage;
#[allow(unused_imports)]
pub use info::{LanguageInfo, ServiceModel, ServicesInfoData, ServicesInfoResponse};
#[allow(unused_imports)]
pub use translate::{TranslationResult, TranslationStatusResponse, TranslationUsage};

use crate::config::API_KEY;
use crate::debug_log;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(crate) const API_BASE_URL: &str = "https://api.opensubtitles.com/api/v1";

/// Shared type for the UI to receive transient status messages from the API
/// client (e.g. "Re-authenticating…"). None = idle/no message.
pub type StatusSink = Arc<Mutex<Option<String>>>;

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

#[derive(Debug, Clone)]
pub struct ApiClient {
    pub(crate) auth_token: String,
    /// Stored credentials so the client can silently re-authenticate when the
    /// server reports the current token as invalid/expired.
    pub(crate) username: Option<String>,
    pub(crate) password: Option<String>,
    /// If set, the client pushes transient human-readable status messages here
    /// (e.g. "Session expired, re-authenticating…") so the UI can surface them
    /// without coupling to the actual call sites.
    pub(crate) status_sink: Option<StatusSink>,
    /// If the client refreshed the token during a call, the new value lands
    /// here so callers can persist it. Cleared by [`take_refreshed_token`].
    pub(crate) refreshed_token: Option<String>,
    pub(crate) client: reqwest::blocking::Client,
}

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

    /// If the client refreshed the token during a call, take ownership of the
    /// new value. Subsequent calls return `None`. The UI uses this to persist
    /// the refreshed token back into plugin settings.
    pub fn take_refreshed_token(&mut self) -> Option<String> {
        self.refreshed_token.take()
    }

    /// Build the common authenticated headers for an API request.
    pub(crate) fn common_headers(&self) -> reqwest::header::HeaderMap {
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

    /// Send an authenticated request, retrying once with a freshly issued
    /// token if the server reports the current token as invalid.
    ///
    /// `op_name` is used purely for logging (e.g. `"get_credits"`).
    /// `build_request` is called at most twice — once with the cached token
    /// and, on a recoverable auth failure, again after `refresh_token`.
    pub(crate) fn send_authed(
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
