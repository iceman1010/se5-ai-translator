//! Authentication: login, token refresh, auth-failure detection.

use super::{ApiClient, TranslateError, API_BASE_URL};
use crate::config::API_KEY;
use crate::debug_log;

#[allow(dead_code)]
#[derive(Debug, Clone, serde::Deserialize)]
pub struct LoginResponse {
    pub token: Option<String>,
}

impl ApiClient {
    /// Authenticate against the API and return a fresh bearer token.
    /// Static method — callable without an existing `ApiClient` instance.
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

    /// Returns whether this client is capable of refreshing its own token
    /// (i.e. credentials are present). Used by [`send_authed`] to decide
    /// whether a 401 is recoverable.
    pub(crate) fn has_credentials(&self) -> bool {
        self.username.as_ref().is_some_and(|u| !u.is_empty())
            && self.password.as_ref().is_some_and(|p| !p.is_empty())
    }

    /// Push a transient status message to the UI sink (if any).
    pub(crate) fn push_status(&self, msg: Option<&str>) {
        if let Some(sink) = &self.status_sink
            && let Ok(mut g) = sink.lock() {
                *g = msg.map(String::from);
            }
    }

    /// Re-authenticate using stored credentials. Updates `self.auth_token` and
    /// records the new value in `self.refreshed_token` for later persistence.
    pub(crate) fn refresh_token(&mut self) -> Result<(), TranslateError> {
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
    pub(crate) fn is_auth_failure(status: reqwest::StatusCode, body: &str) -> bool {
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
}
