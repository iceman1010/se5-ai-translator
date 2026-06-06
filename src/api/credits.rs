//! Credits endpoints: balance + purchase packages.

use super::{ApiClient, TranslateError, API_BASE_URL};
use reqwest::blocking::multipart;
use serde::Deserialize;

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

impl ApiClient {
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
}
